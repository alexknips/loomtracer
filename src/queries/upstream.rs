//! Take it upstream: what the decision a disagreement or supersession lands on rests on
//! (hivemind-ottw.5, design section 7 of hivemind-ottw).
//!
//! `disagree` and `supersede` act on one decision, the leaf. When that decision rests on
//! something, the disagreement may really be with that something: a premise other decisions
//! rest on too. This read gives the write replies the two facts that let the caller notice, so
//! the question can be asked before the leaf is the only thing that changed.
//!
//! - `rests_on`: everything the decision rests on, each item with its standing (the same
//!   [`GroundingItem`] `verify` shows), and for a prior decision how many decisions rest on it.
//! - `raise_upstream`: the prior decisions it rests on that still stand, each with how many
//!   *other* decisions rest on it. A hint. Raising it there is `disagree` or `supersede` on the
//!   premise, which is the caller's call: nothing here writes, replaces or drops the action just
//!   taken on the leaf.
//!
//! Only a prior decision that holds is listed for raising: it is the one premise a `disagree` or
//! `supersede` can act on, and a premise that is already superseded, rejected or contested
//! already has the disagreement on record. An assumption or a bet is shown in `rests_on` with its
//! standing and is answered with evidence, not with a disagreement.
//!
//! Pure Layer-2 read: one hop over explicit premise links, no traversal, no ranking, no model.
//! Counts are the same distinct-decision count `dependents_count` gives, not weighted by status.

use serde::Serialize;

use crate::projector::GraphView;
use crate::Result;

use super::grounding::{
    dependent_decision_ids, grounding_of, GroundingItem, GroundingItemState, GroundingKind,
};

/// One thing the decision rests on, as `verify` shows it, plus how many decisions rest on it.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct UpstreamPremise {
    #[serde(flatten)]
    pub item: GroundingItem,
    /// Distinct decisions that follow from this one, the decision acted on included. Set for a
    /// prior decision; absent for evidence, an assumption or a bet.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dependents_count: Option<usize>,
}

/// A prior decision the acted-on decision rests on that still stands: the disagreement may be
/// with it.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RaiseUpstream {
    pub premise_id: String,
    pub label: String,
    /// Decisions that rest on the premise besides the one acted on (and the one replacing it).
    pub other_dependents: usize,
}

/// The reply fields a disagreement or supersession carries about what its target rests on.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Upstream {
    pub rests_on: Vec<UpstreamPremise>,
    pub raise_upstream: Vec<RaiseUpstream>,
}

/// What `decision_id` rests on and which of it still stands. `replacement` is the decision that
/// now replaces it after a supersession, left out of `other_dependents` because it is the same
/// act, not another decision relying on the premise.
///
/// A decision that is not in the graph rests on nothing declared; callers act on it first.
pub fn upstream_of(
    graph: &impl GraphView,
    decision_id: &str,
    replacement: Option<&str>,
) -> Result<Upstream> {
    let grounding = grounding_of(graph, decision_id)?;
    let mut rests_on = Vec::with_capacity(grounding.items.len());
    let mut raise_upstream = Vec::new();
    for item in grounding.items {
        let dependents = match item.kind {
            GroundingKind::Decision => Some(dependent_decision_ids(graph, &item.id)?),
            GroundingKind::Evidence | GroundingKind::Assumption | GroundingKind::Bet => None,
        };
        if let Some(dependents) = &dependents {
            if item.state == GroundingItemState::Holds {
                let other_dependents = dependents
                    .iter()
                    .filter(|id| id.as_str() != decision_id && Some(id.as_str()) != replacement)
                    .count();
                raise_upstream.push(RaiseUpstream {
                    premise_id: item.id.clone(), // ubs:ignore: owned copy for the hint; the item is moved into rests_on below
                    label: item.label.clone(), // ubs:ignore: owned copy for the hint; the item is moved into rests_on below
                    other_dependents,
                });
            }
        }
        rests_on.push(UpstreamPremise {
            dependents_count: dependents.map(|ids| ids.len()),
            item,
        });
    }
    Ok(Upstream {
        rests_on,
        raise_upstream,
    })
}

#[cfg(test)]
mod tests;
