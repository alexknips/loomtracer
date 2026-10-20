//! What a `link_supersession` call checks before it writes (hivemind-4p0xd).
//!
//! `Commands::supersede_decision` appends one `decision.superseded` for two decisions that are
//! already recorded -- "B supersedes A" without creating B -- and looks at the ledger only: that
//! both ids are recorded and differ. It does not look at the graph, so it would record a
//! supersession that closes a loop, or a second edge for a pair that is already linked. This is
//! the verb-level rule that does, shared by MCP (stdio and HTTP) and REST so the transports
//! cannot drift: the same split `crate::grounding`'s `premise_cycle_refusal` and
//! `crate::reaffirmation` have, since the check needs the graph.
//!
//! Why it exists: `supersede_decision` records the replacement itself, accepted and decided by
//! the caller, so a person's own answer that supersedes an earlier record would be written down
//! as the caller's. Here the newer decision is captured first, with its real decider, and this
//! links the two. The link is attributed to the caller; the newer decision keeps its decider.
//!
//! Pure read of the graph, no model. Nothing is written for a refusal or for a pair that is
//! already linked.
//!
//! - **Refused:** an empty id; the same id twice; an id that is not a recorded decision; a link
//!   whose reverse already holds, directly or through other decisions (the old decision
//!   already supersedes the new one), which would close a loop.
//! - **Already linked:** the new decision already supersedes the old one, directly or through
//!   other decisions. Nothing is written and the reply says so with the chain, so a retry
//!   after a client timeout is safe and a shortcut edge never splits a chain into a fork.

use serde::Serialize;

use crate::commands::{CommandContext, Commands};
use crate::events::{EventId, EventProvenance, TenantId};
use crate::ledger::EventLedger;
use crate::projector::GraphView;
use crate::queries::{get_decision, supersedes_path};
use crate::util::require_non_empty;
use crate::Result;

/// A `link_supersession` call: `new_decision_id` supersedes `old_decision_id`, said by `actor_id`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct SupersessionLinkCall<'a> {
    pub(crate) actor_id: &'a str,
    pub(crate) old_decision_id: &'a str,
    pub(crate) new_decision_id: &'a str,
}

/// The reply to a call that was not refused.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct SupersessionLinkReply {
    pub(crate) old_decision_id: String,
    pub(crate) new_decision_id: String,
    /// `false` when the new decision already superseded the old one: nothing was written.
    pub(crate) linked: bool,
    /// The `decision.superseded` event this call appended; `None` when `linked` is `false`.
    pub(crate) event_id: Option<EventId>,
    /// When `linked` is `false`, the decisions from the new one down to the old one that
    /// already carry the supersession (two ids for a direct link, more for one through others).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) already_linked_through: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum SupersessionLinkOutcome {
    Linked(SupersessionLinkReply),
    /// Why the call was refused, in words that name the decisions; nothing was written.
    Refused(String),
}

/// The one implementation of "B supersedes A, both already recorded" behind both MCP transports
/// and REST: check the call against `graph` (which must be current) and, when it is sound and
/// new, record `decision.superseded` attributed to `call.actor_id`.
pub(crate) fn link_supersession<L: EventLedger>(
    ledger: &L,
    tenant_id: &TenantId,
    provenance: EventProvenance,
    graph: &impl GraphView,
    call: SupersessionLinkCall<'_>,
) -> Result<SupersessionLinkOutcome> {
    let SupersessionLinkCall {
        actor_id,
        old_decision_id,
        new_decision_id,
    } = call;
    require_non_empty("old_decision_id", old_decision_id)?;
    require_non_empty("new_decision_id", new_decision_id)?;

    if old_decision_id == new_decision_id {
        return Ok(SupersessionLinkOutcome::Refused(format!(
            "decision {old_decision_id} cannot supersede itself; nothing was written"
        )));
    }
    for id in [old_decision_id, new_decision_id] {
        if get_decision(graph, id)?.data.is_none() {
            return Ok(SupersessionLinkOutcome::Refused(format!(
                "decision is not recorded: {id}; nothing was written"
            )));
        }
    }

    if let Some(chain) = supersedes_path(graph, old_decision_id, new_decision_id)? {
        let mut message = String::new();
        push_label(&mut message, graph, old_decision_id)?;
        message.push_str(" already supersedes ");
        push_label(&mut message, graph, new_decision_id)?;
        if chain.len() > 2 {
            message.push_str(" (");
            push_chain(&mut message, graph, &chain)?;
            message.push(')');
        }
        message.push_str(
            ", so recording the reverse would close a loop of supersessions; nothing was written",
        );
        return Ok(SupersessionLinkOutcome::Refused(message));
    }

    if let Some(chain) = supersedes_path(graph, new_decision_id, old_decision_id)? {
        return Ok(SupersessionLinkOutcome::Linked(SupersessionLinkReply {
            old_decision_id: old_decision_id.to_owned(),
            new_decision_id: new_decision_id.to_owned(),
            linked: false,
            event_id: None,
            already_linked_through: Some(chain),
        }));
    }

    let commands =
        Commands::new_with_context(ledger, CommandContext::new(tenant_id.clone(), provenance));
    let event_id = commands.supersede_decision(old_decision_id, new_decision_id, actor_id)?;
    Ok(SupersessionLinkOutcome::Linked(SupersessionLinkReply {
        old_decision_id: old_decision_id.to_owned(),
        new_decision_id: new_decision_id.to_owned(),
        linked: true,
        event_id: Some(event_id),
        already_linked_through: None,
    }))
}

/// `"<title>" (<id>)`, or just the id for a decision the graph has no title for.
fn push_label(out: &mut String, graph: &impl GraphView, decision_id: &str) -> Result<()> {
    match get_decision(graph, decision_id)?.data {
        Some(decision) => {
            out.push_str(&format!("\"{}\" ({decision_id})", decision.title));
        }
        None => out.push_str(decision_id),
    }
    Ok(())
}

/// `"A" (a) supersedes "B" (b) supersedes "C" (c)`: the decisions of a supersession chain in order.
fn push_chain(out: &mut String, graph: &impl GraphView, chain: &[String]) -> Result<()> {
    for (index, link) in chain.iter().enumerate() {
        if index > 0 {
            out.push_str(" supersedes ");
        }
        push_label(out, graph, link)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
