//! Option labels of a classified batch: a decision capture whose labels the record must not hold
//! is refused, and the rest of the batch is recorded (hivemind-0921o). The invariants are listed
//! in the `commands` module header.

use std::collections::HashSet;

use serde::Serialize;

use crate::events::CaptureItem;

use super::{option_label_problem, RestatedCapture};

/// One capture of a classification that was not recorded because of an option label.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RefusedCapture {
    /// Position among the captures as submitted.
    pub index: usize,
    pub title: String,
    /// What a capture verb says of the same label.
    pub reason: String,
}

/// What `set_aside_refused` leaves of a classification's captures.
pub(super) struct LabelSettlement {
    /// The captures to go on with, in submission order.
    pub kept: Vec<CaptureItem>,
    /// Each kept capture's position as submitted, in the same order as `kept`.
    pub kept_at: Vec<usize>,
    pub refused: Vec<RefusedCapture>,
}

/// Why a decision capture's labels may not be recorded, or `None` when they may. The weighed
/// options and the chosen option are both labels a reader is shown (`why` prints "chose: ..."),
/// and the projector writes Option nodes for a decision only, so no other kind is asked.
fn capture_label_problem(capture: &CaptureItem) -> Option<String> {
    if capture.kind != "decision" {
        return None;
    }
    capture
        .options
        .iter()
        .flatten()
        .chain(capture.chosen_option.iter())
        .find_map(|label| option_label_problem(label))
}

/// Splits `captures` into those to record and those refused for an option label every capture
/// verb refuses (`validate_option_label`). A refused capture is recorded nowhere, so a kept
/// capture that supersedes it by title loses that reference: the title names nothing.
pub(super) fn set_aside_refused(captures: Vec<CaptureItem>) -> LabelSettlement {
    let mut kept = Vec::with_capacity(captures.len());
    let mut kept_at = Vec::with_capacity(captures.len());
    let mut refused = Vec::new();
    for (index, capture) in captures.into_iter().enumerate() {
        match capture_label_problem(&capture) {
            Some(reason) => refused.push(RefusedCapture {
                index,
                title: capture.title,
                reason,
            }),
            None => {
                kept.push(capture);
                kept_at.push(index);
            }
        }
    }

    if !refused.is_empty() {
        let kept_titles: HashSet<&str> = kept.iter().map(|capture| capture.title.trim()).collect();
        let refused_titles: HashSet<String> = refused
            .iter()
            .map(|capture| capture.title.trim())
            .filter(|title| !kept_titles.contains(title))
            .map(str::to_owned)
            .collect();
        for capture in &mut kept {
            if capture
                .supersedes_id
                .as_deref()
                .is_some_and(|reference| refused_titles.contains(reference.trim()))
            {
                capture.supersedes_id = None;
            }
        }
    }
    LabelSettlement {
        kept,
        kept_at,
        refused,
    }
}

/// `restated` as `settle_restatements` returned it, numbered among the captures kept, put back
/// to their position as submitted, which is what the reply says.
pub(super) fn at_submitted_positions(
    mut restated: Vec<RestatedCapture>,
    kept_at: &[usize],
) -> Vec<RestatedCapture> {
    for capture in &mut restated {
        capture.index = kept_at.get(capture.index).copied().unwrap_or(capture.index);
    }
    restated
}
