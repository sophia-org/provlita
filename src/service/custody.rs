//! Observe every admitted SDK unit before its bounded ticket history expires.
//! Custody is settled per unit; an unknown outcome is fatal and never replayed.
use super::*;
use sophia_shell_client::{Admission, Custody, Ticket};

// The pinned SDK retains 256 tickets. Bound the span, not just the pending
// count: newer completed writes must not evict an older submission whose
// custody is still unknown.
const MAX_TICKET_SPAN: u64 = 128;

#[derive(Default)]
pub(super) struct CustodyWatch {
    pending: VecDeque<Ticket>,
    last: u64,
}
impl CustodyWatch {
    fn reserve(&self, maximum: usize) -> Result<(), ShellClientError> {
        if self
            .pending
            .front()
            .is_some_and(|first| self.last - first.0 + 1 + maximum as u64 > MAX_TICKET_SPAN)
        {
            return Err(ShellClientError::QueueSaturated);
        }
        Ok(())
    }
    fn record(&mut self, admission: Admission) {
        for ticket in admission.tickets() {
            self.last = ticket.0;
            self.pending.push_back(ticket);
        }
    }
    /// Admitted tickets whose custody was still open at the last observation.
    pub(super) fn unsettled(&self) -> usize {
        self.pending.len()
    }
    pub(super) fn observe(&mut self, connection: &ShellConnection) -> Result<(), String> {
        for _ in 0..self.pending.len() {
            let ticket = self.pending.pop_front().expect("bounded ticket count");
            match connection.custody(ticket) {
                Some(Custody::Queued | Custody::InFlight) => self.pending.push_back(ticket),
                Some(Custody::Submitted | Custody::Stored) => {}
                Some(Custody::Written) => {
                    return Err("file submission reported socket-only custody".into());
                }
                Some(Custody::Refused(errno)) => {
                    return Err(format!(
                        "dock submission {} refused: errno={errno}",
                        ticket.0
                    ));
                }
                Some(Custody::Unknown | Custody::DroppedUnsent) => {
                    return Err(format!(
                        "dock submission {} ended without custody",
                        ticket.0
                    ));
                }
                None => return Err("dock submission ticket expired before observation".into()),
            }
        }
        Ok(())
    }
}
impl<R: RasterExecutor> DockService<R> {
    pub(super) fn enqueue_content(
        &mut self,
        tx: TransactionId,
        record: &ShellContentRecord,
    ) -> Result<(), ShellClientError> {
        self.custody.reserve(1)?;
        let admission = self.connection.enqueue_content_tracked(tx, record)?;
        self.custody.record(admission);
        Ok(())
    }
    pub(super) fn enqueue_catalog_candidate(
        &mut self,
        tx: TransactionId,
        begin: &CatalogCandidateBegin,
        chunk: ContentCandidateChunk,
        end: &ContentCandidateEnd,
    ) -> Result<(), ShellClientError> {
        self.custody.reserve(1)?;
        let lifecycle = self.lifecycle.as_mut().expect("limits precede candidates");
        let admission = self.connection.enqueue_catalog_candidate_tracked(
            lifecycle,
            tx,
            begin,
            &[chunk],
            end,
        )?;
        self.custody.record(admission);
        Ok(())
    }
    pub(super) fn enqueue_action_response(
        &mut self,
        tx: TransactionId,
        ack: &ContentActionAck,
        activation: Option<&CatalogActivation>,
    ) -> Result<(), ShellClientError> {
        self.custody
            .reserve(1 + usize::from(activation.is_some()))?;
        let admission = self.connection.enqueue_catalog_action_response_tracked(
            tx,
            ack,
            activation.map(|value| (tx, value)),
        )?;
        self.custody.record(admission);
        Ok(())
    }
}
