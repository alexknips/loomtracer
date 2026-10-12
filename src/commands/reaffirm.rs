//! Reaffirmation: a decision that rests on a premise "still holds" after the premise changed
//! (hivemind-ottw.1).
//!
//! `reaffirm_decision` appends exactly one `decision.reaffirmed {decision_id, premise_id,
//! reason}`, attributed to the actor who looked again. It does not derive, fan out or touch any
//! other decision: whether the pair has an open re-examine item to close is derived from the
//! graph on read, and refusing a reaffirmation that closes nothing is the calling verb's rule,
//! because it needs the graph (the same split `crate::grounding::premise_cycle_refusal` has).
//! The invariants are listed in the `commands` module header.

use uuid::Uuid;

use crate::error::CommandError;
use crate::events::{DecisionReaffirmedPayload, EventId, EventPayload};
use crate::ledger::EventLedger;
use crate::util::{require_non_empty, require_valid_actor_id};
use crate::Result;

use super::{require_not_own_premise, Commands};

/// What a reaffirmation names: `decision_id` is the dependent, `premise_id` the decision or
/// hypothesis it rests on, `reason` the one line for why it still holds.
#[derive(Debug, Clone, Copy)]
pub struct ReaffirmInput<'a> {
    pub actor_id: &'a str,
    pub decision_id: &'a str,
    pub premise_id: &'a str,
    pub reason: &'a str,
}

impl<L: EventLedger> Commands<'_, L> {
    /// Record that `input.decision_id` still holds after `input.premise_id` changed.
    pub fn reaffirm_decision(&self, input: ReaffirmInput<'_>) -> Result<EventId> {
        self.reaffirm_decision_with_uuid(input, Uuid::new_v4())
    }

    /// `reaffirm_decision` with the event's uuid chosen by the caller, so a transport that
    /// retries (an HTTP client that did not see the reply) gets the existing event id back
    /// from the ledger instead of a second event.
    ///
    /// There is deliberately no check for an earlier reaffirmation by the same actor with the
    /// same words. A reaffirmation closes the item as of its own ledger offset, and the premise
    /// may have changed again since: the same person saying the same thing after that change
    /// is a new act, and returning the old event id would leave the reopened item open while
    /// the caller is told it was closed.
    pub fn reaffirm_decision_with_uuid(
        &self,
        input: ReaffirmInput<'_>,
        event_uuid: Uuid,
    ) -> Result<EventId> {
        let ReaffirmInput {
            actor_id,
            decision_id,
            premise_id,
            reason,
        } = input;
        require_valid_actor_id(actor_id)?;
        require_non_empty("decision_id", decision_id)?;
        require_non_empty("premise_id", premise_id)?;
        require_non_empty("reason", reason)?;
        require_not_own_premise(decision_id, premise_id)?;

        self.require_decision_exists(decision_id)?;
        if !self.decision_exists(premise_id)? && !self.hypothesis_exists(premise_id)? {
            return Err(CommandError::Invariant(format!(
                "premise does not exist: {premise_id} (neither a decision nor a hypothesis)"
            ))
            .into());
        }

        let event = self.event_with_uuid(
            actor_id,
            EventPayload::DecisionReaffirmed(DecisionReaffirmedPayload {
                decision_id: decision_id.to_owned(),
                premise_id: premise_id.to_owned(),
                reason: reason.to_owned(),
            }),
            None,
            event_uuid,
        )?;

        self.append_event(event)
    }
}

#[cfg(test)]
mod tests;
