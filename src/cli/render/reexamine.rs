//! Text and JSON for `loomtracer reaffirm` and `loomtracer reexamine` (hivemind-ottw.4).
//!
//! The words are the ones the re-examine query already states (a change is `superseded`,
//! `rejected`, `contested` or `refuted`; an item is open, reaffirmed, or closed because the
//! dependent stopped standing); nothing here derives or ranks anything.

use std::fmt::Write as _;

use crate::queries::{
    ChangeKind, ItemState, PremiseChange, ReaffirmationSummary, ReexamineItem, ReexamineResults,
};
use crate::reaffirmation::{AmbiguousPremise, ReaffirmReply};
use crate::Result;

use super::format_json_value;

/// `reaffirm`'s reply: the decision that still holds, the premise, and the event recorded.
pub(crate) fn format_reaffirm_output(as_json: bool, reply: &ReaffirmReply) -> Result<String> {
    if as_json {
        return format_json_value(true, reply);
    }
    let mut rendered = format!(
        "reaffirmed \"{}\" ({}): still holds after \"{}\" ({})",
        reply.decision_title, reply.decision_id, reply.premise_label, reply.premise_id
    );
    if let Some(change) = reply.covers.first() {
        let _ = write!(rendered, " was {}", change_phrase(change));
    }
    let _ = write!(
        rendered,
        "\n  recorded as event {} by {}: \"{}\"",
        reply.event_id, reply.actor_id, reply.reason
    );
    Ok(rendered)
}

/// The listing a call with several open items and no `--premise` gets instead of a write.
pub(crate) fn format_reaffirm_ambiguous_output(
    as_json: bool,
    ambiguous: &AmbiguousPremise,
) -> Result<String> {
    if as_json {
        return format_json_value(true, &ambiguous.envelope());
    }
    let mut rendered = format!(
        "\"{}\" ({}) has {} open re-examine items, and nothing was written. Re-run with --premise naming one (its id, or words from its title):",
        ambiguous.decision_title,
        ambiguous.decision_id,
        ambiguous.items.len()
    );
    for item in &ambiguous.items {
        let _ = write!(
            rendered,
            "\n  {}  \"{}\" was {}",
            item.entry.premise_id,
            item.entry.premise_label,
            change_phrase(&item.entry.change)
        );
    }
    Ok(rendered)
}

/// The `reexamine` listing, one line per item, then what a reader needs to trust the list:
/// the scope it was asked in and what it did not name.
pub(crate) fn render_reexamine_summary(results: &ReexamineResults, include_closed: bool) -> String {
    let mut lines: Vec<String> = Vec::new();
    lines.push(match (results.items.len(), include_closed) {
        (0, false) => "no open re-examine items".to_owned(),
        (0, true) => "no re-examine items".to_owned(),
        (count, false) => format!("{count} open re-examine item(s)"),
        (count, true) => format!("{count} re-examine item(s)"),
    });
    lines.extend(
        results
            .items
            .iter()
            .map(|item| format!("  {}", item_line(item))),
    );
    if let Some(scope) = &results.scope {
        lines.push(format!("scope: {}", scope.note));
    }
    if let Some(not_shown) = results.not_shown.filter(|count| *count > 0) {
        lines.push(format!(
            "{not_shown} more item(s) are on decisions outside this project's scope (counted, not named)"
        ));
    }
    lines.join("\n")
}

fn item_line(item: &ReexamineItem) -> String {
    let entry = &item.entry;
    let mut line = format!(
        "{} \"{}\" ({})",
        state_word(&entry.state),
        item.dependent.title,
        item.dependent.id
    );
    if item.dependent.project.is_some() {
        let _ = write!(line, " [{}]", item.dependent.project_label);
    }
    let _ = write!(
        line,
        " on \"{}\" ({}) {}",
        entry.premise_label,
        entry.premise_id,
        change_phrase(&entry.change)
    );
    if let Some(label) = &item.premise_project_label {
        if item.premise_project != item.dependent.project {
            let _ = write!(line, " (premise from {label})");
        }
    }
    match &entry.state {
        ItemState::Open => {}
        ItemState::Reaffirmed { reaffirmations } => {
            if let Some(first) = reaffirmations.first() {
                let _ = write!(line, "; {}", reaffirmation_phrase(first));
                if reaffirmations.len() > 1 {
                    let _ = write!(line, " (+{} more)", reaffirmations.len() - 1);
                }
            }
        }
        ItemState::DependentSuperseded { by_id } => {
            let _ = write!(line, "; the decision was superseded by {by_id}");
        }
        ItemState::DependentRejected => line.push_str("; the decision was rejected"),
    }
    if let Some(earlier) = entry.earlier_reaffirmations.last() {
        let _ = write!(
            line,
            "; reopened: {} came before this change",
            reaffirmation_phrase(earlier)
        );
    }
    line
}

fn state_word(state: &ItemState) -> &'static str {
    match state {
        ItemState::Open => "OPEN",
        ItemState::Reaffirmed { .. } => "REAFFIRMED",
        ItemState::DependentSuperseded { .. } => "CLOSED (superseded)",
        ItemState::DependentRejected => "CLOSED (rejected)",
    }
}

/// `superseded 2026-09-02 by d:new`, `rejected by human:bob`, `contested (dissent from
/// human:bob)`, `refuted 2026-09-05 by e:1`: how the premise changed, when the graph says, and by
/// whom or what.
pub(crate) fn change_phrase(change: &PremiseChange) -> String {
    let when = change
        .at_time
        .map(|at| format!(" {}", at.format("%Y-%m-%d")))
        .unwrap_or_default();
    let by = change.by_ids.join(", ");
    match change.kind {
        ChangeKind::Superseded => format!("superseded{when} by {by}"),
        ChangeKind::Rejected => format!("rejected by {by}"),
        ChangeKind::Contested => format!("contested (dissent from {by})"),
        ChangeKind::Refuted => format!("refuted{when} by {by}"),
    }
}

fn reaffirmation_phrase(reaffirmation: &ReaffirmationSummary) -> String {
    let when = reaffirmation
        .at
        .map(|at| format!(" on {}", at.format("%Y-%m-%d")))
        .unwrap_or_default();
    format!(
        "reaffirmed by {}{when}: \"{}\"",
        reaffirmation.actor_id, reaffirmation.reason
    )
}
