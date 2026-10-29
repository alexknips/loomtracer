//! Attribution dispute: the actor a decision names as its decider says the call was never theirs
//! (hivemind-purmd), directly or through their agent (hivemind-f9xgu).
//!
//! `dispute_attribution` appends exactly one `decision.attribution_disputed {decision_id,
//! reason}`, attributed to the disputing actor. It does not touch the acceptance it disputes,
//! reject the decision or change its status: a person who never made a call is not rejecting it,
//! and `disagree` refuses an actor that is recorded as having accepted. The dispute sits beside
//! the acceptance and the read side shows both. `relay_attribution_dispute` is the same event
//! recorded by an agent for the person, with the person's verbatim words and where they said
//! them. The invariants are listed in the `commands` module header.

use uuid::Uuid;

use crate::error::CommandError;
use crate::events::{DecisionAttributionDisputedPayload, EventId, EventPayload, EventType};
use crate::ledger::EventLedger;
use crate::util::{require_non_empty, require_valid_actor_id};
use crate::Result;

use super::{classified_decision_capture, json_string_list, same_identifier, Commands, Relay};

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
    /// An agent that is not a decider is told the paths it can take instead
    /// ([`not_a_decider_refusal`]).
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
            return Err(not_a_decider_refusal(actor_id, decision_id).into());
        }

        let event = self.event_with_uuid(
            actor_id,
            EventPayload::DecisionAttributionDisputed(DecisionAttributionDisputedPayload {
                decision_id: decision_id.to_owned(),
                reason: reason.to_owned(),
                said_in: None,
            }),
            None,
            event_uuid,
        )?;

        self.append_event(event)
    }

    /// Record `disputed_by`'s dispute of their own attribution on `decision_id`, as their agent
    /// `relayed_by` heard it (hivemind-f9xgu): the same `decision.attribution_disputed` a person
    /// writes with their own credential, and the same shape as a decision an agent records for a
    /// person with `decided_by` and a quote. `quote` is the person's verbatim words, kept as the
    /// event's `reason`; `said_in` is the session, thread or message they said them in, when the
    /// agent knows.
    pub fn relay_attribution_dispute(
        &self,
        relayed_by: &str,
        disputed_by: &str,
        decision_id: &str,
        quote: &str,
        said_in: Option<&str>,
    ) -> Result<EventId> {
        self.relay_attribution_dispute_with_uuid(
            relayed_by,
            disputed_by,
            decision_id,
            quote,
            said_in,
            Uuid::new_v4(),
        )
    }

    /// `relay_attribution_dispute` with the event's uuid chosen by the caller, so a transport
    /// that retries gets the existing event id back from the ledger instead of a second event.
    ///
    /// The event's actor is `disputed_by` and its provenance is `agent` with `relayed_by` as the
    /// source reference, whatever provenance this `Commands` was built with: a relayed dispute
    /// can never be recorded without the relay. Only a person may be relayed (`disputed_by` is a
    /// `human:` actor) and only by an agent (`relayed_by` is an `agent:` actor); an agent that
    /// is itself named as a decider disputes with its own credential, as before. The person has
    /// to be an actor the decision names as its decider, the same rule as a direct dispute, so
    /// an agent cannot put a dispute under the name of someone the decision never named.
    pub fn relay_attribution_dispute_with_uuid(
        &self,
        relayed_by: &str,
        disputed_by: &str,
        decision_id: &str,
        quote: &str,
        said_in: Option<&str>,
        event_uuid: Uuid,
    ) -> Result<EventId> {
        require_non_empty("decision_id", decision_id)?;
        let relay = Relay::new(relayed_by, said_in);
        relay.require_valid(disputed_by, quote, "dispute")?;
        self.require_decision_exists(decision_id)?;

        if !self.is_recorded_decider(decision_id, disputed_by)? {
            return Err(CommandError::Invariant(format!(
                "{disputed_by} is not recorded as a decider of decision {decision_id}, so \
                 {relayed_by} cannot record a dispute of that attribution for them: only an \
                 actor the decision names as its decider can say the call was not theirs. To \
                 record that they disagree with the decision, use `disagree_decision`."
            ))
            .into());
        }

        let event = self.relayed_event(
            &relay,
            disputed_by,
            EventPayload::DecisionAttributionDisputed(DecisionAttributionDisputedPayload {
                decision_id: decision_id.to_owned(),
                reason: quote.to_owned(),
                said_in: relay.recorded_said_in(),
            }),
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

/// The refusal for a dispute from an actor the decision does not name as its decider
/// (hivemind-9vrok). A person, or anything else on its own credential, is pointed at `disagree`.
/// An agent is not: it reaches here by following a hint that told it to dispute with `--reason`,
/// and for the person it is relaying `disagree` is the path that refuses it in turn, so this
/// text names what the agent is missing (`disputed_by` and the person's `quote`) and the other
/// paths it can take, as tools (MCP is where an agent reaches the cell) with the CLI forms
/// beside them, the way [`Relay::already_accepted_refusal`] does for the reverse refusal.
fn not_a_decider_refusal(actor_id: &str, decision_id: &str) -> CommandError {
    let refused = format!(
        "{actor_id} is not recorded as a decider of decision {decision_id}, so it cannot dispute \
         that attribution: only an actor the decision names as its decider can say the call was \
         not theirs, and only for itself."
    );
    if !actor_id.starts_with("agent:") {
        return CommandError::Invariant(format!(
            "{refused} To say you disagree with the decision, use `loomtracer disagree`."
        ));
    }
    CommandError::Invariant(format!(
        "{refused} If a person told you the call was never theirs and someone else recorded \
         their name against it, record that for them with `dispute_attribution` (`disputed_by` \
         the person as `human:<name>` and their verbatim words as `quote`; CLI: `loomtracer \
         dispute-attribution --decision {decision_id} --disputed-by human:<name> --quote ...`): \
         it is kept beside the acceptance, which stays. To record that someone disagrees with \
         the decision itself, use `disagree_decision` (CLI: `loomtracer disagree`): with \
         `disagreed_by` and their `quote` for a person who did not accept it, with your own \
         `reason` for yourself. To replace it, capture the replacement with `capture_decision`, \
         then `link_supersession` from {decision_id} to it."
    ))
}

#[cfg(test)]
mod tests;
