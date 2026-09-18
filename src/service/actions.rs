//! Exact Presented/Action enters Xilem once, then one owned reply retries only
//! FIFO admission. The widget intent is never an independent launch permission.
use super::*;
impl<R: RasterExecutor> DockService<R> {
    pub(super) fn action(&mut self, action: ContentAction, eligible: bool) -> Result<(), String> {
        if self.replies.len() >= self.limits.as_ref().unwrap().max_pending_actions as usize {
            return Err("dock action credit exhausted".into());
        }
        let view = self
            .outputs
            .iter()
            .find(|output| output.facts.output == action.output)
            .and_then(|output| output.presented.as_ref());
        let intent = if eligible && action.kind == 1 {
            view.and_then(|view| self.views.activate(view, action.target_id))
        } else {
            None
        };
        let activation = intent
            .filter(|intent| {
                intent.catalog_generation == self.catalog_generation
                    && intent.catalog_generation == action.interaction_generation
                    && u64::from(intent.slot) == action.action_id
            })
            .map(|intent| CatalogActivation {
                catalog_generation: intent.catalog_generation,
                action: action.clone(),
            });
        let ack = ContentActionAck {
            grant: action.grant,
            output: action.output,
            candidate_generation: action.candidate_generation,
            presentation_epoch: action.presentation_epoch,
            interaction_generation: action.interaction_generation,
            allocation: action.allocation,
            target_id: action.target_id,
            target_generation: action.target_generation,
            action_id: action.action_id,
            event_id: action.event_id,
            disposition: if activation.is_some() { 1 } else { 2 },
        };
        let transaction = self.tx()?;
        self.replies.push_back(ActionReply {
            transaction,
            ack,
            activation,
            sent: false,
            deadline: Instant::now() + RESPONSE_TIMEOUT,
        });
        Ok(())
    }
    pub(super) fn flush_actions(&mut self) -> Result<(), String> {
        let mut index = 0;
        while index < self.replies.len() {
            let reply = &self.replies[index];
            if Instant::now() >= reply.deadline {
                return Err("dock action response timed out".into());
            }
            if reply.sent {
                index += 1;
                continue;
            }
            match self.connection.enqueue_catalog_action_response(
                reply.transaction,
                &reply.ack,
                reply
                    .activation
                    .as_ref()
                    .map(|value| (reply.transaction, value)),
            ) {
                Ok(()) => {}
                Err(ShellClientError::QueueSaturated) => break,
                Err(e) => return Err(e.to_string()),
            }
            if reply.activation.is_some() {
                self.replies[index].sent = true;
                index += 1;
            } else {
                let reply = self.replies.remove(index).unwrap();
                self.lifecycle
                    .as_mut()
                    .unwrap()
                    .finish_action(reply.ack.event_id);
            }
        }
        Ok(())
    }
}
