//! FIFO observations update the same lifecycle used by generic shell clients.
use super::*;
use sophia_shell_client::ContentActionDispatch;

impl<R: RasterExecutor> DockService<R> {
    pub(super) fn observe(&mut self, observation: CatalogObservation) -> Result<usize, String> {
        let (tx, record) = match observation {
            CatalogObservation::Catalog(_, catalog) => {
                let generation = catalog.catalog.generation;
                self.views.install_catalog(catalog)?;
                self.catalog_generation = generation;
                for output in &mut self.outputs {
                    output.dirty = true;
                }
                return Ok(0);
            }
            CatalogObservation::Outcome(tx, outcome) => {
                let position = self
                    .replies
                    .iter()
                    .position(|reply| {
                        reply.sent
                            && reply.transaction == tx
                            && reply.activation.as_ref() == Some(&outcome.activation)
                    })
                    .ok_or("unmatched catalog activation outcome")?;
                let reply = self.replies.remove(position).unwrap();
                self.lifecycle
                    .as_mut()
                    .unwrap()
                    .finish_action(reply.ack.event_id);
                println!(
                    "provlita_action schema=1 event_id={} status={} catalog_generation={}",
                    reply.ack.event_id, outcome.status, outcome.activation.catalog_generation
                );
                return Ok(0);
            }
            CatalogObservation::Content(tx, record) => (tx, record),
        };
        if let ShellContentRecord::Limits(limits) = &record {
            if self.limits.is_some()
                || limits.grant.connection_epoch != self.connection.connection_epoch()
            {
                return Err("duplicate or wrong-epoch content limits".into());
            }
            self.lifecycle =
                Some(ContentLifecycle::new(limits.clone()).map_err(|e| format!("limits: {e:?}"))?);
            self.limits = Some(limits.clone());
            return Ok(0);
        }
        let dispatch = self
            .lifecycle
            .as_mut()
            .ok_or("content arrived before limits")?
            .dispatch(tx, record)
            .map_err(|e| format!("ordered content lifecycle: {e:?}"))?;
        let limits = self.limits.as_ref().unwrap();
        match dispatch.record {
            ShellContentRecord::OutputFacts(facts) => {
                if let Some(old) = &self.facts {
                    if old != &facts {
                        return Err(
                            "output topology changed; fresh connection and allocations required"
                                .into(),
                        );
                    }
                    return Ok(0);
                }
                for entry in &facts.outputs {
                    if matches!(&self.config.outputs, Outputs::Selected(ids) if !ids.contains(&entry.output.id))
                    {
                        continue;
                    }
                    if self.outputs.len() >= 16
                        || self.outputs.len()
                            >= limits.max_outputs.min(limits.max_allocations_total) as usize
                    {
                        return Err("dock output inventory exceeds negotiated budget".into());
                    }
                    self.outputs.push(Output {
                        facts: entry.clone(),
                        request: None,
                        allocation: None,
                        slots: [Slot::default(), Slot::default()],
                        current: None,
                        pending: None,
                        presented: None,
                        dirty: true,
                    });
                }
                if self.outputs.is_empty() {
                    return Err("no configured dock output is available".into());
                }
                self.facts = Some(facts);
            }
            ShellContentRecord::AllocationResult(allocation) => {
                let index = usize::try_from(allocation.allocation_request_id)
                    .ok()
                    .and_then(|v| v.checked_sub(1))
                    .ok_or("unknown allocation request")?;
                let output = self
                    .outputs
                    .get_mut(index)
                    .ok_or("unknown allocation output")?;
                if output.request != Some(tx)
                    || output.allocation.is_some()
                    || output.facts.output != allocation.output
                    || allocation.status != 1
                    || allocation.reason != 0
                    || allocation.scale_denominator == 0
                    || allocation.scale_generation != output.facts.scale_generation
                    || allocation.pixel.height > self.allowance
                    || allocation.allowed_reservation_extent < allocation.pixel.height
                {
                    return Err("dock allocation refused or inconsistent with request".into());
                }
                output.allocation = Some(allocation);
            }
            ShellContentRecord::ResourceStatus(status) => {
                let output = self
                    .outputs
                    .iter_mut()
                    .find(|output| {
                        output.pending.as_ref().is_some_and(|pending| {
                            output.slots[pending.slot].resource == status.resource
                        })
                    })
                    .ok_or("resource status names no pending upload")?;
                let pending = output.pending.as_mut().unwrap();
                if pending.upload_tx != tx || status.reason != 0 {
                    return Err("resource upload refused or wrong transaction".into());
                }
                match (pending.phase, status.status) {
                    (Phase::WaitBegin, 1) => pending.phase = Phase::Chunks,
                    (Phase::WaitEnd, 2) => {
                        pending.bytes = Vec::new();
                        pending.phase = Phase::Demand;
                        output.slots[pending.slot].state = ResourceState::Resident;
                    }
                    _ => return Err("unexpected resource acknowledgement phase".into()),
                }
                pending.deadline = Instant::now() + RESPONSE_TIMEOUT;
            }
            ShellContentRecord::FramePermit(permit) => {
                let output = self
                    .outputs
                    .iter_mut()
                    .find(|output| output.facts.output == permit.output)
                    .ok_or("permit names another output")?;
                let pending = output.pending.as_mut().ok_or("permit without demand")?;
                if pending.phase != Phase::WaitPermit
                    || pending.demand_tx != tx
                    || pending.demand != permit.demand_id
                    || permit.state != 1
                    || permit.reason != 0
                    || permit.permit_id == 0
                    || permit.ttl_ms == 0
                {
                    return Err("pacing permit refused or mismatched".into());
                }
                pending.permit = Some((permit, Instant::now()));
                pending.phase = Phase::Candidate;
            }
            ShellContentRecord::CandidateOutcome(outcome) => {
                let output = self
                    .outputs
                    .iter_mut()
                    .find(|output| output.facts.output == outcome.output)
                    .ok_or("candidate outcome names another output")?;
                let pending = output
                    .pending
                    .as_ref()
                    .ok_or("candidate outcome without owner")?;
                if pending.phase != Phase::WaitCandidate
                    || pending.candidate_tx != tx
                    || pending.candidate != outcome.candidate_generation
                    || outcome.reason != 0
                {
                    return Err(format!(
                        "dock candidate rejected or mismatched: {}",
                        outcome.reason
                    ));
                }
                match outcome.kind {
                    1 => {}
                    2 => {
                        let pending = output.pending.take().unwrap();
                        if let Some(old) = output.current {
                            output.slots[old].state = ResourceState::RetireNeeded;
                        }
                        output.current = Some(pending.slot);
                        output.presented = Some(pending.view);
                        println!(
                            "provlita_candidate schema=1 status=presented connection_epoch={} output={} candidate_generation={} presentation_epoch={} catalog_generation={}",
                            limits.grant.connection_epoch,
                            outcome.output.id,
                            outcome.candidate_generation,
                            outcome.presentation_epoch,
                            output.presented.as_ref().unwrap().catalog_generation
                        );
                        return Ok(1);
                    }
                    _ => return Err("dock candidate not presented".into()),
                }
            }
            ShellContentRecord::ResourceReleased(released) => {
                let slot = self
                    .outputs
                    .iter_mut()
                    .flat_map(|o| &mut o.slots)
                    .find(|s| s.resource == released.resource)
                    .ok_or("release names no owned resource")?;
                if slot.state != ResourceState::Retiring || released.reason != 0 {
                    return Err("release did not match exact retirement".into());
                }
                slot.resource.generation = slot
                    .resource
                    .generation
                    .checked_add(1)
                    .ok_or("resource generation exhausted")?;
                slot.state = ResourceState::Free;
                slot.bytes = 0;
                slot.deadline = None;
            }
            ShellContentRecord::Action(action) => {
                if dispatch.action == Some(ContentActionDispatch::Cancelled) {
                    self.replies
                        .retain(|reply| reply.ack.event_id != action.event_id || reply.sent);
                } else {
                    self.action(
                        action,
                        dispatch.action == Some(ContentActionDispatch::Eligible),
                    )?;
                }
            }
            _ => return Err("unexpected server dock content record".into()),
        }
        Ok(0)
    }
}
