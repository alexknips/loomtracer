//! Attribution dispute: the actor a decision names as its decider says the call was never theirs
//! (hivemind-purmd).
//!
//! `dispute_attribution` appends exactly one `decision.attribution_disputed {decision_id,
//! reason}`, attributed to the disputing actor. It does not touch the acceptance it disputes,
//! reject the decision or change its status: a person who never made a call is not rejecting it,
//! and `disagree` refuses an actor that is recorded as having accepted. The dispute sits beside
//! the acceptance and the read side shows both. The invariants are listed in the `commands`
//! module header.

use uuid::Uuid;

use crate::error::CommandError;
use crate::events::{DecisionAttributionDisputedPayload, EventId, EventPayload, EventType};
use crate::ledger::EventLedger;
use crate::util::{require_non_empty, require_valid_actor_id};
use crate::Result;

use super::{classified_decision_capture, json_string_list, same_identifier, Commands};

impl<L: EventLedger> Commands<'_, L> {
    /// Record that `actor_id`, named as a decider of `decision_id`, did not make that call.
    pub fn dispute_attribution(
        &self,
        actor_id: &str,
        decision_id: &str,
        reason: &str,
    ) -> Result<EventId> {
        self.dispute_attribution_with_uuid(actor_id, decision_id, reason, Uuid::new_v4())
    }

    /// `dispute_attribution` with the event's uuid chosen by the caller, so a transport that
    /// retries gets the existing event id back from the ledger instead of a second event.
    ///
    /// Only an actor the decision names as a decider may dispute it, and only for itself: the
    /// event's actor is the one disputing, and nothing here takes a target. Someone who merely
    /// disagrees with the decision is not refused here but pointed at `disagree`, because they
    /// are not a decider and the dispute would say something about a name that is not theirs.
    pub fn dispute_attribution_with_uuid(
        &self,
        actor_id: &str,
        decision_id: &str,
        reason: &str,
        event_uuid: Uuid,
    ) -> Result<EventId> {
        require_valid_actor_id(actor_id)?;
        require_non_empty("decision_id", decision_id)?;
        require_non_empty("reason", reason)?;
        self.require_decision_exists(decision_id)?;

        if !self.is_recorded_decider(decision_id, actor_id)? {
            return Err(CommandError::Invariant(format!(
                "{actor_id} is not recorded as a decider of decision {decision_id}, so it cannot \
                 dispute that attribution: only an actor the decision names as its decider can \
                 say the call was not theirs, and only for itself. To say you disagree with the \
                 decision, use `loomtracer disagree`."
            ))
            .into());
        }

        let event = self.event_with_uuid(
            actor_id,
            EventPayload::DecisionAttributionDisputed(DecisionAttributionDisputedPayload {
                decision_id: decision_id.to_owned(),
                reason: reason.to_owned(),
            }),
            None,
            event_uuid,
        )?;

        self.append_event(event)
    }

    /// Whether the ledger names `actor_id` as a decider of `decision_id`: an acceptance it wrote
    /// (`decision.accepted`, including the one an agent records for a person with `decided_by`),
    /// or, for a decision a classifier drafted from a transcript, an id the capture lists in
    /// `accepted_by`.
    fn is_recorded_decider(&self, decision_id: &str, actor_id: &str) -> Result<bool> {
        if self.actor_has_decision_event(decision_id, actor_id, EventType::DecisionAccepted)? {
            return Ok(true);
        }
        Ok(self
            .classified_batch_for(decision_id)?
            .and_then(|(batch, index)| {
                classified_decision_capture(&batch, index)
                    .map(|capture| json_string_list(capture, "accepted_by"))
            })
            .is_some_and(|deciders| {
                deciders
                    .iter()
                    .any(|decider| same_identifier(decider, actor_id))
            }))
    }
}

#[cfg(test)]
mod tests;
