//! What a `reaffirm` call checks before it writes (hivemind-ottw.4).
//!
//! `Commands::reaffirm_decision` appends one `decision.reaffirmed` and looks at nothing else: it
//! does not even check that the decision rests on the premise. This is the verb-level rule that
//! does, shared by the CLI, MCP (stdio and HTTP) and REST so the four cannot drift: a
//! reaffirmation is recorded only against an item that is **open** right now (the decision rests on
//! a premise that changed, and nobody has said it still holds since). Anything else is refused,
//! and nothing is written, in words that say why. The same split `crate::grounding`'s
//! `premise_cycle_refusal` has: the graph is needed, so the calling verb applies it.
//!
//! Pure read, no model: the items come from the re-examine query (`queries::get_reexamine_items`)
//! and the premise a caller names is matched against that decision's own items, never against the
//! whole graph.
//!
//! - **One open item** and no premise named: that is the one. **Several**: the call is ambiguous,
//!   the open items are listed and nothing is written (a premise is never guessed).
//! - **A premise named** (its id, or words from its title or statement): the open item it names.
//!   Nothing open under that name is a refusal, and a refusal for a closed item names who closed
//!   it, when and with which event, so a retried call sees what happened. There is deliberately no
//!   identical-retry dedupe in the command (`reaffirm_decision_with_uuid` has the reason): the
//!   retry-safe call is the one that carries its own event uuid, and this check is what turns a
//!   plain re-run into a refusal rather than a second event.

use serde::Serialize;
use serde_json::{json, Value};

use crate::commands::{CommandContext, Commands, ReaffirmInput, Relay};
use crate::error::CommandError;
use crate::events::{EventId, EventProvenance, TenantId};
use crate::ledger::EventLedger;
use crate::projector::GraphView;
use crate::queries::{
    get_decision, get_reexamine_items, grounding_of, is_sole_superseder, ChangeKind,
    DecisionStatus, DecisionView, GroundingItem, GroundingKind, ItemState, PremiseChange,
    PremiseKind, QueryContext, ReaffirmationSummary, ReexamineItem, ReexamineRequest,
};
use crate::Result;

/// The one open item a call will close: what the reaffirmation names, and the change it covers.
#[derive(Debug, Clone)]
pub(crate) struct ReaffirmTarget {
    pub(crate) decision_id: String,
    pub(crate) decision_title: String,
    pub(crate) premise_id: String,
    pub(crate) premise_kind: PremiseKind,
    pub(crate) premise_label: String,
    /// The change of the premise as the check saw it. A change that lands after the check is not
    /// covered by what the reply says it covered; the reply lists this one by its offset.
    pub(crate) covers: PremiseChange,
}

/// More than one open item and nothing narrowed it down: the caller picks.
#[derive(Debug, Clone)]
pub(crate) struct AmbiguousPremise {
    pub(crate) decision_id: String,
    pub(crate) decision_title: String,
    pub(crate) items: Vec<ReexamineItem>,
}

impl AmbiguousPremise {
    /// The MCP / REST success-shaped result: the resolver envelope every fluent tool returns,
    /// with the open items as the candidates. No event was appended.
    pub(crate) fn envelope(&self) -> Value {
        json!({
            "result_count": self.items.len(),
            "truncated": false,
            "latency_ms": 0,
            "data": {
                "outcome": "ambiguous",
                "field": "premise",
                "decision_id": self.decision_id,
                "decision_title": self.decision_title,
                "items": self.items,
            },
        })
    }
}

/// What the check decided. Nothing has been written for any of them.
#[derive(Debug, Clone)]
pub(crate) enum ReaffirmPlan {
    /// Exactly one open item is named: record the reaffirmation against it.
    Ready(Box<ReaffirmTarget>),
    /// Several open items match and the call did not say which.
    Ambiguous(AmbiguousPremise),
    /// There is nothing to re-examine, or the premise is not one the decision rests on. The text
    /// says why and ends with "nothing was written".
    Refused(String),
}

/// Check a reaffirmation of `decision_id`, optionally naming the premise (`premise`: its id, or
/// words from its title or statement), against the graph as it stands.
pub(crate) fn plan_reaffirmation(
    context: &QueryContext,
    graph: &impl GraphView,
    ledger: &impl EventLedger,
    decision_id: &str,
    premise: Option<&str>,
) -> Result<ReaffirmPlan> {
    // The same error, and so the same exit code and HTTP 404, every verb gives a decision id
    // nothing has.
    let Some(decision) = get_decision(graph, decision_id)?.data else {
        return Err(
            CommandError::Invariant(format!("decision does not exist: {decision_id}")).into(),
        );
    };
    let premise = premise.map(str::trim);
    if premise == Some("") {
        return Ok(refused(
            "the premise to reaffirm is empty: name the decision or assumption that changed, or leave it out when only one has"
                .to_owned(),
        ));
    }

    let items = items_of(context, graph, ledger, decision_id)?;
    let named: Vec<&ReexamineItem> = match premise {
        None => items.iter().collect(),
        Some(premise) => named_items(&items, premise),
    };
    let open: Vec<&ReexamineItem> = named
        .iter()
        .copied()
        .filter(|item| item.entry.state.is_open())
        .collect();

    match open.as_slice() {
        [item] => Ok(ReaffirmPlan::Ready(Box::new(ReaffirmTarget {
            decision_id: decision.id.clone(),
            decision_title: decision.title.clone(),
            premise_id: item.entry.premise_id.clone(),
            premise_kind: item.entry.premise_kind,
            premise_label: item.entry.premise_label.clone(),
            covers: item.entry.change.clone(),
        }))),
        [] => Ok(refused(nothing_to_reexamine(
            graph, &decision, premise, &named,
        )?)),
        _ => Ok(ReaffirmPlan::Ambiguous(AmbiguousPremise {
            decision_id: decision.id,
            decision_title: decision.title,
            items: open.into_iter().cloned().collect(),
        })),
    }
}

fn refused(reason: String) -> ReaffirmPlan {
    ReaffirmPlan::Refused(format!("{reason}; nothing was written"))
}

/// Every item (open and closed) of the decision. A decision rests on a handful of premises, so
/// this is one page; it follows the cursor anyway rather than ever cutting the list short.
fn items_of(
    context: &QueryContext,
    graph: &impl GraphView,
    ledger: &impl EventLedger,
    decision_id: &str,
) -> Result<Vec<ReexamineItem>> {
    let mut request = ReexamineRequest {
        dependent_id: Some(decision_id.to_owned()),
        include_closed: true,
        limit: usize::MAX,
        ..ReexamineRequest::default()
    };
    let mut items = Vec::new();
    loop {
        let page = get_reexamine_items(context, graph, ledger, &request)?;
        items.extend(page.data.items);
        match page.data.next_cursor {
            Some(next) => request.cursor = Some(next),
            None => return Ok(items),
        }
    }
}

/// The items whose premise the caller named: the premise id exactly, else the items whose
/// title or statement holds every word given (any case).
fn named_items<'a>(items: &'a [ReexamineItem], premise: &str) -> Vec<&'a ReexamineItem> {
    let by_id: Vec<&ReexamineItem> = items
        .iter()
        .filter(|item| item.entry.premise_id == premise)
        .collect();
    if !by_id.is_empty() {
        return by_id;
    }
    items
        .iter()
        .filter(|item| holds_every_word(&item.entry.premise_label, premise))
        .collect()
}

fn holds_every_word(text: &str, words: &str) -> bool {
    let text = text.to_lowercase();
    words
        .split_whitespace()
        .all(|word| text.contains(&word.to_lowercase()))
}

/// Why a call found no open item, in words that say what is true of the decision and the
/// premise it named (or of the decision, when it named none).
fn nothing_to_reexamine(
    graph: &impl GraphView,
    decision: &DecisionView,
    premise: Option<&str>,
    named: &[&ReexamineItem],
) -> Result<String> {
    let title = quoted(&decision.title);
    if !named.is_empty() {
        let closed: Vec<String> = named
            .iter()
            .map(|item| closed_phrase(&title, item))
            .collect();
        return Ok(format!("nothing to re-examine: {}", closed.join("; ")));
    }
    if matches!(
        decision.status,
        DecisionStatus::Superseded | DecisionStatus::Rejected
    ) {
        let status = match decision.status {
            DecisionStatus::Superseded => "superseded",
            _ => "rejected",
        };
        return Ok(format!(
            "nothing to re-examine: {title} is {status} and no longer stands"
        ));
    }

    let rests_on: Vec<_> = grounding_of(graph, &decision.id)?
        .items
        .into_iter()
        .filter(|item| item.kind != GroundingKind::Evidence)
        .collect();
    let Some(premise) = premise else {
        let mut alone = Vec::new();
        for item in &rests_on {
            if superseded_by_it_alone(graph, decision, item)? {
                alone.push(quoted(&item.label));
            }
        }
        if !alone.is_empty() {
            let others = rests_on.len() - alone.len();
            let rest = if others == 0 {
                String::new()
            } else {
                format!("; none of the {others} other premise(s) it rests on has changed")
            };
            return Ok(format!(
                "nothing to re-examine: {title} is the decision that superseded {}, so there is nothing for it to re-examine about {}{rest}",
                alone.join(" and "),
                if alone.len() == 1 { "it" } else { "them" }
            ));
        }
        return Ok(if rests_on.is_empty() {
            format!(
                "nothing to re-examine: {title} rests on no decision, assumption or bet, so no premise of it can have changed"
            )
        } else {
            format!(
                "nothing to re-examine: none of the {} premise(s) {title} rests on has changed",
                rests_on.len()
            )
        });
    };
    match rests_on
        .iter()
        .find(|item| item.id == premise || holds_every_word(&item.label, premise))
    {
        // The premise did change: this decision is what changed it, so it has no item on it.
        Some(item) if superseded_by_it_alone(graph, decision, item)? => Ok(format!(
            "nothing to re-examine: {title} is the decision that superseded {}, so there is nothing for it to re-examine about it",
            quoted(&item.label)
        )),
        Some(item) => Ok(format!(
            "nothing to re-examine: {} has not changed, so {title} has nothing to reaffirm about it",
            quoted(&item.label)
        )),
        None => {
            let listing = if rests_on.is_empty() {
                "it rests on no decision, assumption or bet".to_owned()
            } else {
                let labels: Vec<String> = rests_on
                    .iter()
                    .map(|item| format!("{} ({})", quoted(&item.label), item.id))
                    .collect();
                format!("it rests on {}", labels.join(", "))
            };
            Ok(format!(
                "{title} does not rest on \"{premise}\"; {listing}"
            ))
        }
    }
}

/// Whether `item`, something `decision` rests on, is a decision that `decision` alone superseded:
/// the revision is the change, so it has no item on it (the rule re-examine items follow).
fn superseded_by_it_alone(
    graph: &impl GraphView,
    decision: &DecisionView,
    item: &GroundingItem,
) -> Result<bool> {
    Ok(item.kind == GroundingKind::Decision && is_sole_superseder(graph, &decision.id, &item.id)?)
}

/// One closed item, as a clause of a refusal. A reaffirmation names who recorded it, when, and
/// its event id; a dependent that stopped standing names how.
fn closed_phrase(title: &str, item: &ReexamineItem) -> String {
    let premise = quoted(&item.entry.premise_label);
    let change = change_word(item.entry.change.kind);
    match &item.entry.state {
        ItemState::Reaffirmed { reaffirmations } => {
            let closer = reaffirmations
                .first()
                .map(describe_reaffirmation)
                .unwrap_or_default();
            let later = match reaffirmations.len() {
                0 | 1 => String::new(),
                more => format!(" and {} more time(s) since", more - 1),
            };
            format!(
                "{title} was already reaffirmed after its latest change of {premise} ({change}), by {closer}{later}"
            )
        }
        ItemState::DependentSuperseded { by_id } => format!(
            "{title} was itself superseded (by {by_id}) after {premise} was {change}, which closed the item"
        ),
        ItemState::DependentRejected => format!(
            "{title} was itself rejected after {premise} was {change}, which closed the item"
        ),
        ItemState::Open => format!("{title} still has an open item on {premise}"),
    }
}

/// `human:alex on 2026-10-13 10:42 UTC (event 42, id <uuid>, "the reason")`: who, when, and the
/// event, by the ledger number the original reply gave as `event_id` and by its own uuid.
fn describe_reaffirmation(reaffirmation: &ReaffirmationSummary) -> String {
    let when = reaffirmation
        .at
        .map(|at| format!(" on {}", at.format("%Y-%m-%d %H:%M UTC")))
        .unwrap_or_default();
    let number = reaffirmation
        .event_origin
        .map(|origin| format!("event {origin}, "))
        .unwrap_or_default();
    format!(
        "{}{when} ({number}id {}, \"{}\")",
        reaffirmation.said_by(),
        reaffirmation.id,
        reaffirmation.reason
    )
}

fn change_word(kind: ChangeKind) -> &'static str {
    match kind {
        ChangeKind::Superseded => "superseded",
        ChangeKind::Rejected => "rejected",
        ChangeKind::Contested => "contested",
        ChangeKind::Refuted => "refuted",
    }
}

fn quoted(text: &str) -> String {
    format!("\"{text}\"")
}

/// What a recorded reaffirmation reads back as, on every transport: the decision that still
/// holds, the premise it still holds after, the event, and exactly which change it covered.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct ReaffirmReply {
    pub(crate) decision_id: String,
    pub(crate) decision_title: String,
    pub(crate) premise_id: String,
    pub(crate) premise_kind: PremiseKind,
    pub(crate) premise_label: String,
    pub(crate) event_id: EventId,
    /// Who it is attributed to.
    pub(crate) actor_id: String,
    pub(crate) reason: String,
    /// The agent that recorded the reaffirmation for the person it is attributed to
    /// (hivemind-efo6x). Absent when the actor reaffirmed with their own credential.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) relayed_by: Option<String>,
    /// Where the person said it, when the relaying agent stated it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) said_in: Option<String>,
    /// The change(s) of the premise this reaffirmation answers, by ledger offset. A change
    /// recorded after it is a new item, never covered by it.
    pub(crate) covers: Vec<PremiseChange>,
}

/// How a `reaffirm` call ended. `Recorded` is the only one that wrote anything.
#[derive(Debug, Clone)]
pub(crate) enum ReaffirmOutcome {
    Recorded(Box<ReaffirmReply>),
    Ambiguous(AmbiguousPremise),
    Refused(String),
}

/// What a caller says: who, which decision, optionally which premise, and why. With `relay`,
/// `actor_id` is the person and `reason` their verbatim words, recorded by the agent `relay`
/// names (hivemind-efo6x); without it they are the actor's own.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ReaffirmCall<'a> {
    pub(crate) actor_id: &'a str,
    pub(crate) decision_id: &'a str,
    pub(crate) premise: Option<&'a str>,
    pub(crate) reason: &'a str,
    pub(crate) relay: Option<Relay<'a>>,
}

/// The one implementation of "reaffirm" behind the CLI, both MCP transports and REST: check the
/// call against `graph` (which must be current), and when exactly one open item is named, record
/// the reaffirmation attributed to `call.actor_id`. A blank reason is refused before anything is
/// read. Nothing is written for `Ambiguous` or `Refused`.
pub(crate) fn reaffirm<L: EventLedger>(
    ledger: &L,
    tenant_id: &TenantId,
    provenance: EventProvenance,
    graph: &impl GraphView,
    call: ReaffirmCall<'_>,
) -> Result<ReaffirmOutcome> {
    let ReaffirmCall {
        actor_id,
        decision_id,
        premise,
        reason,
        relay,
    } = call;
    // Before anything is read, so a relay that cannot be recorded is refused as that and never
    // answered with an ambiguity about the premise.
    if let Some(relay) = &relay {
        relay.require_valid(actor_id, reason, "reaffirmation")?;
    }
    crate::util::require_non_empty("reason", reason)?;
    // One line, recorded as typed: the edges of it are not part of what someone said.
    let reason = reason.trim();

    let context = QueryContext::new(tenant_id.clone());
    let target = match plan_reaffirmation(&context, graph, ledger, decision_id, premise)? {
        ReaffirmPlan::Ready(target) => *target,
        ReaffirmPlan::Ambiguous(ambiguous) => return Ok(ReaffirmOutcome::Ambiguous(ambiguous)),
        ReaffirmPlan::Refused(text) => return Ok(ReaffirmOutcome::Refused(text)),
    };

    let commands =
        Commands::new_with_context(ledger, CommandContext::new(tenant_id.clone(), provenance));
    let input = ReaffirmInput {
        actor_id,
        decision_id: &target.decision_id,
        premise_id: &target.premise_id,
        reason,
    };
    let event_id = match &relay {
        Some(relay) => commands.relay_reaffirmation(relay.by, relay.said_in, input)?,
        None => commands.reaffirm_decision(input)?,
    };

    Ok(ReaffirmOutcome::Recorded(Box::new(ReaffirmReply {
        decision_id: target.decision_id,
        decision_title: target.decision_title,
        premise_id: target.premise_id,
        premise_kind: target.premise_kind,
        premise_label: target.premise_label,
        event_id,
        actor_id: actor_id.to_owned(),
        reason: reason.to_owned(),
        relayed_by: relay.as_ref().map(|relay| relay.by.to_owned()),
        said_in: relay.as_ref().and_then(Relay::recorded_said_in),
        covers: vec![target.covers],
    })))
}

#[cfg(test)]
mod tests;
