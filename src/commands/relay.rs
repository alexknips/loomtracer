//! A person's act recorded by their agent (hivemind-f9xgu, hivemind-efo6x, hivemind-fsxx9).
//!
//! People use Loomtracer through their agents, so an agent that hears a person say something
//! about a decision (the call was never theirs, they disagree, it still holds, yes go with that)
//! records it as the person's own, the way `decided_by` and a quote record a decision they made:
//! the event's actor is the person, its provenance is `agent` with the relaying agent as
//! `source_ref` (whatever provenance the `Commands` carries, so a relayed act can never be
//! recorded without the relay), and the person's verbatim words are in the event (the `reason` of
//! a dispute, disagreement or reaffirmation, the `quote` of an acceptance), with `said_in` saying
//! where they said them. This module holds what the verbs that do it share: who may be relayed
//! and by whom, and how the event is built.

use chrono::Utc;
use uuid::Uuid;

use crate::error::CommandError;
use crate::events::{
    DecisionAcceptedPayload, Event, EventBuilder, EventId, EventPayload, EventProvenance, EventType,
};
use crate::ledger::EventLedger;
use crate::util::{require_non_empty, require_valid_actor_id};
use crate::Result;

use super::{already_rejected_refusal, Commands};

/// The agent that heard a person say something and records it for them.
#[derive(Debug, Clone, Copy)]
pub struct Relay<'a> {
    /// The agent (`agent:<tool>:<name>`) that recorded the words: the event's `source_ref`.
    pub by: &'a str,
    /// The session, thread or message the person said them in. `None` when the agent does not
    /// know: it is never guessed.
    pub said_in: Option<&'a str>,
}

impl<'a> Relay<'a> {
    pub fn new(by: &'a str, said_in: Option<&'a str>) -> Self {
        Self { by, said_in }
    }

    /// `said_in` as it is recorded: trimmed, and absent when the relay gave none.
    pub(crate) fn recorded_said_in(&self) -> Option<String> {
        self.said_in.map(|said_in| said_in.trim().to_owned())
    }

    /// Refuse a relay that is not an agent recording a person's words. `what` names the act in
    /// the refusal ("dispute", "disagreement", "reaffirmation"). Only a person may be relayed
    /// (`person` is a `human:` actor) and only by an agent (`by` is an `agent:` actor); an agent
    /// that speaks for itself uses its own credential, as before. The words (`quote`) are
    /// required: a person's name without their words is not a relay.
    pub(crate) fn require_valid(&self, person: &str, quote: &str, what: &str) -> Result<()> {
        require_valid_actor_id(self.by)?;
        require_valid_actor_id(person)?;
        require_non_empty("quote", quote)?;
        if self
            .said_in
            .is_some_and(|said_in| said_in.trim().is_empty())
        {
            return Err(CommandError::Validation(
                "said_in must not be blank: name the session, thread or message the words come \
                 from, or leave it out"
                    .to_owned(),
            )
            .into());
        }
        if !self.by.starts_with("agent:") {
            return Err(CommandError::Validation(format!(
                "{} cannot relay a {what}: only an agent records a person's words for them. A \
                 person records their own {what} with their own credential, without a quote.",
                self.by
            ))
            .into());
        }
        if !person
            .strip_prefix("human:")
            .is_some_and(|name| !name.trim().is_empty())
        {
            return Err(CommandError::Validation(format!(
                "{person} is not a person (human:<name>): an agent relays a person's words, and \
                 an agent that speaks for itself uses its own credential."
            ))
            .into());
        }
        Ok(())
    }
}

/// What a person accepted and what they said: `accepted_by` is the person (`human:<name>`),
/// `decision_id` the decision they accepted, `quote` their verbatim words and `question` what
/// they were answering.
#[derive(Debug, Clone, Copy)]
pub struct AcceptanceInput<'a> {
    pub accepted_by: &'a str,
    pub decision_id: &'a str,
    pub quote: &'a str,
    pub question: &'a str,
}

impl<L: EventLedger> Commands<'_, L> {
    /// Record that `input.accepted_by`, a person, accepted `input.decision_id` in the words
    /// `input.quote`, said in answer to `input.question`, as their agent `relayed_by` heard it
    /// (hivemind-fsxx9): "yes, go with that" about a decision that was left open. `said_in` is
    /// the session, thread or message they said it in, when the agent knows.
    pub fn relay_acceptance(
        &self,
        relayed_by: &str,
        said_in: Option<&str>,
        input: AcceptanceInput<'_>,
    ) -> Result<EventId> {
        self.relay_acceptance_with_uuid(relayed_by, said_in, input, Uuid::new_v4())
    }

    /// `relay_acceptance` with the event's uuid chosen by the caller, so a transport that
    /// retries gets the existing event id back from the ledger instead of a second event.
    ///
    /// The event is a `decision.accepted` whose actor is the person and whose provenance is
    /// `agent` with `relayed_by` as the source reference, whatever provenance this `Commands`
    /// was built with. Only a person may be relayed (a `human:` actor) and only by an agent (an
    /// `agent:` actor); their words and the question they answered are required. The rules
    /// `accept_decision` enforces hold unchanged: the decision exists and the person has not
    /// rejected it. Nothing here asks whether the decision is still open: a person may accept
    /// what an agent accepted alone, and a second person may accept what a first did, each with
    /// their own words.
    pub fn relay_acceptance_with_uuid(
        &self,
        relayed_by: &str,
        said_in: Option<&str>,
        input: AcceptanceInput<'_>,
        event_uuid: Uuid,
    ) -> Result<EventId> {
        let AcceptanceInput {
            accepted_by,
            decision_id,
            quote,
            question,
        } = input;
        let relay = Relay::new(relayed_by, said_in);
        relay.require_valid(accepted_by, quote, "acceptance")?;
        require_non_empty("decision_id", decision_id)?;
        // Words with no stated question are unreadable once the conversation is gone;
        // `events::validate` pairs them again on read.
        require_non_empty("question", question)?;
        self.require_decision_exists(decision_id)?;
        if self.actor_has_decision_event(decision_id, accepted_by, EventType::DecisionRejected)? {
            return Err(already_rejected_refusal(accepted_by, decision_id).into());
        }

        let event = self.relayed_event(
            &relay,
            accepted_by,
            EventPayload::DecisionAccepted(DecisionAcceptedPayload {
                decision_id: decision_id.to_owned(),
                delegated_by: None,
                quote: Some(quote.to_owned()),
                question: Some(question.to_owned()),
                said_in: relay.recorded_said_in(),
            }),
            event_uuid,
        )?;

        self.append_event(event)
    }

    /// The event as `person` said it and `relay.by` recorded it: the person is the actor and the
    /// relaying agent is the provenance, not the `Commands`' own.
    pub(super) fn relayed_event(
        &self,
        relay: &Relay<'_>,
        person: &str,
        payload: EventPayload,
        event_uuid: Uuid,
    ) -> Result<Event> {
        EventBuilder::new(event_uuid, person, payload)
            .tenant_id(self.context.tenant_id.clone())
            .provenance(EventProvenance::agent(relay.by))
            .timestamp(Some(self.context.event_ts.unwrap_or_else(Utc::now)))
            .build()
            .map_err(|error| {
                CommandError::Invariant(format!("failed to build typed event envelope: {error}"))
                    .into()
            })
    }
}

#[cfg(test)]
mod tests;
