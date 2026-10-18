//! Per-output upload, pacing and presentation obligations. One output's retained
//! ResourceReleased never becomes a global scheduling gate.
use super::*;
use sophia_shell_protocol::encoding::catalog_actions::encode_catalog_content_candidate;

#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum Phase {
    Begin,
    WaitImage,
    Demand,
    WaitPermit,
    Candidate,
    WaitCandidate,
}
pub(super) struct Pending {
    pub slot: usize,
    pub view: ViewMetadata,
    pub bytes: Vec<u8>,
    /// The sealed source, retained only while `image_begin` refuses
    /// it with Busy; the producer owns it after admission.
    pub source: Option<std::os::fd::OwnedFd>,
    pub offer: Option<u64>,
    pub accepted: bool,
    pub demand_tx: TransactionId,
    pub candidate_tx: TransactionId,
    pub demand: u64,
    pub permit: Option<(ContentFramePermit, Instant)>,
    pub candidate: u64,
    pub phase: Phase,
    pub deadline: Instant,
}
impl<R: RasterExecutor> DockService<R> {
    pub(super) fn finish_render(
        &mut self,
        rendering: Rendering,
        mut result: RenderResult,
    ) -> Result<(), String> {
        let output = &mut self.outputs[rendering.output];
        let allocation = output
            .allocation
            .as_ref()
            .ok_or("rendered allocation vanished")?;
        if (result.width, result.height) != (allocation.pixel.width, allocation.pixel.height)
            || result.bytes.len() as u64 != output.slots[rendering.slot].bytes
        {
            return Err("GPU result dimensions changed".into());
        }
        if rendering.view.catalog_generation != self.catalog_generation {
            output.slots[rendering.slot].state = ResourceState::Free;
            output.slots[rendering.slot].bytes = 0;
            output.dirty = true;
            return Ok(());
        }
        // Pinned imaging readback is straight RGBA. Wire format 1 is
        // premultiplied BGRA, rounded in nonlinear sRGB, with transparent black.
        for pixel in result.bytes.chunks_exact_mut(4) {
            let a = u16::from(pixel[3]);
            let pm = |v: u8| ((u16::from(v) * a + 127) / 255) as u8;
            let r = pm(pixel[0]);
            pixel[0] = pm(pixel[2]);
            pixel[1] = pm(pixel[1]);
            pixel[2] = r;
        }
        self.outputs[rendering.output].pending = Some(Pending {
            slot: rendering.slot,
            view: rendering.view,
            bytes: result.bytes,
            source: None,
            offer: None,
            accepted: false,
            demand_tx: TransactionId::from_raw(0),
            candidate_tx: TransactionId::from_raw(0),
            demand: 0,
            permit: None,
            candidate: 0,
            phase: Phase::Begin,
            deadline: Instant::now() + RESPONSE_TIMEOUT,
        });
        Ok(())
    }
    pub(super) fn advance(&mut self, index: usize) -> Result<(), String> {
        let Some(pending) = &self.outputs[index].pending else {
            return Ok(());
        };
        if Instant::now() >= pending.deadline {
            return Err("dock upload/presentation response timed out".into());
        }
        let limits = self.limits.as_ref().expect("limits").clone();
        let allocation = self.outputs[index]
            .allocation
            .as_ref()
            .expect("allocation")
            .clone();
        let phase = pending.phase;
        let slot = pending.slot;
        let mut resource = self.outputs[index].slots[slot].resource;
        match phase {
            Phase::Begin => {
                // No image moves before the sealed channel is Ready;
                // a closed channel is an explicit session failure,
                // never a byte-upload fallback.
                let Some(adapter) = self.connection.image_adapter() else {
                    return Err("sealed image channel was not enabled".into());
                };
                match adapter.channel() {
                    Some(sophia_shell_client::ImageChannelState::Ready) => {}
                    Some(sophia_shell_client::ImageChannelState::Closed) => {
                        return Err("sealed image channel closed before Source".into());
                    }
                    _ => return Ok(()),
                }
                let first = resource.id == 0;
                if first {
                    resource.id = self.resource;
                }
                let next = self
                    .resource
                    .checked_add(1)
                    .ok_or("resource IDs exhausted")?;
                let pending = self.outputs[index].pending.as_mut().unwrap();
                if pending.source.is_none() {
                    let width = allocation.pixel.width;
                    let height = allocation.pixel.height;
                    let isb = sophia_image_producer::create_isb(width, height)
                        .map_err(|e| format!("sealed source create: {e:?}"))?;
                    isb.write_pixels(0, &pending.bytes)
                        .map_err(|e| format!("sealed source fill: {e:?}"))?;
                    sophia_image_producer::finish_isb(&isb)
                        .map_err(|e| format!("sealed source seal: {e:?}"))?;
                    pending.source = Some(isb.into_fd());
                }
                let source = pending.source.take().unwrap();
                let metadata = sophia_shell_client::ProducerMetadata {
                    offer_id: 0,
                    resource: resource.id,
                    resource_generation: resource.generation,
                    admission_epoch: limits.grant.content_grant_epoch,
                    output_id: allocation.output.id,
                    allocation_id: allocation.allocation.id,
                    topology_generation: self.facts.as_ref().expect("facts").facts_generation,
                    source_kind: 1,
                    source_format: 1,
                    width: allocation.pixel.width,
                    height: allocation.pixel.height,
                    stride: allocation.pixel.width * 4,
                    plane_count: 1,
                    descriptor_count: 1,
                    declared_bytes: pending.bytes.len() as u64,
                    modifier: 0,
                };
                match self
                    .connection
                    .image_begin(metadata, source, Instant::now())
                {
                    Ok(admission) => {
                        self.custody.record_ticket(admission.ticket);
                        if first {
                            self.resource = next;
                        }
                        self.outputs[index].slots[slot].resource = resource;
                        let pending = self.outputs[index].pending.as_mut().unwrap();
                        pending.offer = Some(admission.offer);
                        pending.phase = Phase::WaitImage;
                        pending.deadline = Instant::now() + RESPONSE_TIMEOUT;
                    }
                    Err(error) => {
                        use sophia_shell_client::ImageAdapterError;
                        if matches!(
                            error.error,
                            ImageAdapterError::Busy | ImageAdapterError::Again
                        ) {
                            // The descriptor came back; retry next turn.
                            self.outputs[index].pending.as_mut().unwrap().source = error.source;
                            return Ok(());
                        }
                        return Err(format!("sealed image begin refused: {:?}", error.error));
                    }
                }
            }
            Phase::WaitImage => {
                let pending = self.outputs[index].pending.as_ref().unwrap();
                let offer = pending.offer.ok_or("image offer was not recorded")?;
                let info = self
                    .connection
                    .image_adapter()
                    .and_then(|adapter| adapter.offer(offer))
                    .ok_or("image offer vanished before settlement")?;
                use sophia_shell_client::OfferState;
                match info.state {
                    OfferState::Settled => {
                        if !pending.accepted
                            || info.status
                                != Some(sophia_image_producer::ResourceStatusKind::Accepted)
                            || info.disposition != 1
                        {
                            return Err(
                                "image settled without Accepted and Settled disposition 1".into()
                            );
                        }
                        self.connection
                            .image_forget(offer)
                            .map_err(|e| format!("image forget: {e:?}"))?;
                        let pending = self.outputs[index].pending.as_mut().unwrap();
                        pending.bytes = Vec::new();
                        pending.phase = Phase::Demand;
                        pending.deadline = Instant::now() + RESPONSE_TIMEOUT;
                        self.outputs[index].slots[slot].state = ResourceState::Resident;
                    }
                    OfferState::Refused
                    | OfferState::Ambiguous
                    | OfferState::Expired
                    | OfferState::Unknown => {
                        return Err(format!(
                            "image offer ended in {:?} before settlement",
                            info.state
                        ));
                    }
                    _ => {}
                }
            }
            Phase::Demand => {
                let tx = self.tx()?;
                if self.enqueue(
                    tx,
                    ShellContentRecord::FrameDemand(ContentFrameDemand {
                        grant: limits.grant,
                        output: allocation.output,
                        allocation: allocation.allocation,
                        demand_id: tx.raw(),
                        reason: 1,
                    }),
                )? {
                    let pending = self.outputs[index].pending.as_mut().unwrap();
                    pending.demand = tx.raw();
                    pending.demand_tx = tx;
                    pending.phase = Phase::WaitPermit;
                }
            }
            Phase::Candidate => self.candidate_enqueue(index)?,
            _ => {}
        }
        Ok(())
    }
    fn candidate_enqueue(&mut self, index: usize) -> Result<(), String> {
        let tx = self.tx()?;
        let limits = self.limits.as_ref().unwrap();
        let output = &self.outputs[index];
        let allocation = output.allocation.as_ref().unwrap();
        let pending = output.pending.as_ref().unwrap();
        let (permit, received) = pending
            .permit
            .as_ref()
            .ok_or("candidate lost pacing permit")?;
        if received.elapsed() >= Duration::from_millis(u64::from(permit.ttl_ms)) {
            return Err("dock pacing permit expired before enqueue".into());
        }
        let candidate = self.candidate;
        let next = candidate.checked_add(1).ok_or("candidate IDs exhausted")?;
        let targets = pending
            .view
            .targets
            .iter()
            .map(|target| ContentTarget {
                surface_index: 0,
                action_kind: 3,
                target_id: target.id,
                target_generation: pending.view.catalog_generation,
                action_id: u64::from(target.slot),
                bounds_px: target.rect,
            })
            .collect::<Vec<_>>();
        if targets.len() > limits.max_candidate_targets as usize {
            return Err("dock exceeds negotiated targets".into());
        }
        let begin = CatalogCandidateBegin {
            catalog_generation: pending.view.catalog_generation,
            content: ContentCandidateBegin {
                grant: limits.grant,
                output: allocation.output,
                candidate_generation: candidate,
                facts_generation: self.facts.as_ref().unwrap().facts_generation,
                pacing_permit: permit.permit_id,
                interaction_generation: pending.view.catalog_generation,
                surface_count: 1,
                placement_count: 1,
                target_count: targets.len() as u32,
            },
        };
        let chunk = ContentCandidateChunk {
            grant: limits.grant,
            candidate_generation: candidate,
            chunk_ordinal: 0,
            surfaces: vec![ContentSurface {
                allocation: allocation.allocation,
                scale_generation: allocation.scale_generation,
                role: 1,
                edge: 3,
                margins: allocation.margins,
                reservation_extent: allocation.pixel.height,
                parent_surface_index: u16::MAX,
                anchor_parent_rect: ContentPixelRect::default(),
            }],
            placements: vec![ContentPlacement {
                resource: output.slots[pending.slot].resource,
                surface_index: 0,
                destination_x_px: 0,
                destination_y_px: 0,
            }],
            targets,
        };
        let end = ContentCandidateEnd {
            grant: limits.grant,
            candidate_generation: candidate,
            surface_count: 1,
            placement_count: 1,
            target_count: chunk.targets.len() as u32,
        };
        // The native file record carries the whole candidate as one value; it
        // stays within both the immutable profile and this permit.
        let candidate_bytes = encode_catalog_content_candidate(&CatalogContentCandidate {
            candidate: ContentCandidate {
                grant: begin.content.grant,
                candidate_generation: candidate,
                output: begin.content.output,
                facts_generation: begin.content.facts_generation,
                pacing_permit: begin.content.pacing_permit,
                interaction_generation: begin.content.interaction_generation,
                surfaces: chunk.surfaces.clone(),
                placements: chunk.placements.clone(),
                targets: chunk.targets.clone(),
            },
            catalog_generation: begin.catalog_generation,
        })
        .map_err(|e| format!("{e:?}"))?
        .len();
        if candidate_bytes > limits.max_candidate_bytes.min(permit.max_candidate_bytes) as usize {
            return Err("dock candidate exceeds negotiated byte budget".into());
        }
        match self.enqueue_catalog_candidate(tx, &begin, chunk, &end) {
            Ok(()) => {}
            Err(ShellClientError::QueueSaturated) => return Ok(()),
            Err(e) => return Err(e.to_string()),
        }
        self.candidate = next;
        let pending = self.outputs[index].pending.as_mut().unwrap();
        pending.candidate = candidate;
        pending.candidate_tx = tx;
        pending.phase = Phase::WaitCandidate;
        pending.deadline = Instant::now() + RESPONSE_TIMEOUT;
        Ok(())
    }
    pub(super) fn retire(&mut self, index: usize) -> Result<(), String> {
        for slot in 0..2 {
            let owned = &self.outputs[index].slots[slot];
            if owned
                .deadline
                .is_some_and(|deadline| Instant::now() >= deadline)
            {
                return Err("resource retirement timed out".into());
            }
            if owned.state != ResourceState::RetireNeeded {
                continue;
            }
            let limits = self.limits.as_ref().unwrap();
            let retiring: u64 = self
                .outputs
                .iter()
                .flat_map(|o| &o.slots)
                .filter(|s| s.state == ResourceState::Retiring)
                .map(|s| s.bytes)
                .sum();
            if retiring + owned.bytes > limits.max_retiring_bytes {
                continue;
            }
            let record = ShellContentRecord::ResourceRetire(ContentResourceRetire {
                grant: limits.grant,
                resource: owned.resource,
            });
            let tx = self.tx()?;
            if self.enqueue(tx, record)? {
                let owned = &mut self.outputs[index].slots[slot];
                owned.state = ResourceState::Retiring;
                owned.deadline = Some(Instant::now() + RESPONSE_TIMEOUT);
            }
        }
        Ok(())
    }
}
