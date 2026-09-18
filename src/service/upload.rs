//! Per-output upload, pacing and presentation obligations. One output's retained
//! ResourceReleased never becomes a global scheduling gate.
use super::*;

#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum Phase {
    Begin,
    WaitBegin,
    Chunks,
    WaitEnd,
    Demand,
    WaitPermit,
    Candidate,
    WaitCandidate,
}
pub(super) struct Pending {
    pub slot: usize,
    pub view: ViewMetadata,
    pub bytes: Vec<u8>,
    pub chunk_bytes: usize,
    pub chunk_count: u32,
    pub chunk: u32,
    pub upload_tx: TransactionId,
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
        let limits = self.limits.as_ref().expect("limits");
        let row = result.width as usize * 4;
        let usable = limits
            .max_frame_payload
            .checked_sub(48)
            .ok_or("frame prefix exceeds payload")?
            .min(limits.max_chunk_bytes) as usize;
        let chunk_bytes = usable / row * row;
        if chunk_bytes == 0 {
            return Err("negotiated chunk cannot hold a row".into());
        }
        let upload_tx = self.tx()?;
        self.outputs[rendering.output].pending = Some(Pending {
            slot: rendering.slot,
            view: rendering.view,
            chunk_count: result.bytes.len().div_ceil(chunk_bytes) as u32,
            bytes: result.bytes,
            chunk_bytes,
            chunk: 0,
            upload_tx,
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
    pub(super) fn advance(&mut self, index: usize, chunks: &mut usize) -> Result<(), String> {
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
                let first = resource.id == 0;
                if first {
                    resource.id = self.resource;
                }
                let next = self
                    .resource
                    .checked_add(1)
                    .ok_or("resource IDs exhausted")?;
                let tx = pending.upload_tx;
                let record = ShellContentRecord::ResourceBegin(ContentResourceBegin {
                    grant: limits.grant,
                    resource,
                    width_px: allocation.pixel.width,
                    height_px: allocation.pixel.height,
                    rendered_scale_numerator: allocation.scale_numerator,
                    rendered_scale_denominator: allocation.scale_denominator,
                    pixel_format: 1,
                    chunk_count: pending.chunk_count,
                    total_bytes: pending.bytes.len() as u64,
                });
                if self.enqueue(tx, record)? {
                    if first {
                        self.resource = next;
                    }
                    self.outputs[index].slots[slot].resource = resource;
                    self.outputs[index].pending.as_mut().unwrap().phase = Phase::WaitBegin;
                }
            }
            Phase::Chunks => {
                while *chunks > 0 {
                    let pending = self.outputs[index].pending.as_ref().unwrap();
                    if pending.chunk == pending.chunk_count {
                        break;
                    }
                    let start = pending.chunk as usize * pending.chunk_bytes;
                    let end = (start + pending.chunk_bytes).min(pending.bytes.len());
                    let record = ShellContentRecord::ResourceChunk(ContentResourceChunk {
                        grant: limits.grant,
                        resource,
                        ordinal: pending.chunk,
                        offset: start as u64,
                        bytes: pending.bytes[start..end].to_vec(),
                    });
                    let tx = pending.upload_tx;
                    if !self.enqueue(tx, record)? {
                        return Ok(());
                    }
                    self.outputs[index].pending.as_mut().unwrap().chunk += 1;
                    *chunks -= 1;
                }
                let pending = self.outputs[index].pending.as_ref().unwrap();
                if pending.chunk == pending.chunk_count {
                    let (tx, bytes, count) = (
                        pending.upload_tx,
                        pending.bytes.len() as u64,
                        pending.chunk_count,
                    );
                    if self.enqueue(
                        tx,
                        ShellContentRecord::ResourceEnd(ContentResourceEnd {
                            grant: limits.grant,
                            resource,
                            total_bytes: bytes,
                            chunk_count: count,
                        }),
                    )? {
                        self.outputs[index].pending.as_mut().unwrap().phase = Phase::WaitEnd;
                    }
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
        // Encoded candidate bytes remain subordinate to both the immutable
        // profile and this permit; the generic FIFO separately reserves space.
        let wire_bytes = encode_shell_catalog_action_frame(
            tx,
            &ShellCatalogActionRecord::CandidateBegin(begin.clone()),
        )
        .map_err(|e| format!("{e:?}"))?
        .len()
            + encode_shell_catalog_action_frame(
                tx,
                &ShellCatalogActionRecord::CandidateChunk(chunk.clone()),
            )
            .map_err(|e| format!("{e:?}"))?
            .len()
            + encode_shell_content_frame(tx, &ShellContentRecord::CandidateEnd(end.clone()))
                .map_err(|e| format!("{e:?}"))?
                .len();
        if wire_bytes > limits.max_candidate_bytes.min(permit.max_candidate_bytes) as usize {
            return Err("dock candidate exceeds negotiated byte budget".into());
        }
        match self.connection.enqueue_catalog_candidate(
            self.lifecycle.as_mut().unwrap(),
            tx,
            &begin,
            &[chunk],
            &end,
        ) {
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
