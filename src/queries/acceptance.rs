//! What a person said when their agent recorded their acceptance of a decision (hivemind-fsxx9).
//!
//! A `decision.accepted` that carries a quote projects to an `Acceptance` node beside the
//! `ACCEPTED_BY` edge it belongs to (see `projector::NodeKind::Acceptance`). This reads those
//! nodes back for one decision: pure, no ranking, nothing inferred. A person who accepts twice
//! has said two things and both are listed, in the order they were recorded; the ledger is the
//! record and this is a reading of it.

use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::projector::{GraphView, NodeKind};
use crate::Result;

use super::shared::{
    node_rows, optional_datetime, optional_int, optional_string, relayer_of, required_string,
};

/// One acceptance a person made in their own words, as their agent recorded it.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct AcceptanceStatement {
    pub accepted_by: String,
    /// Their verbatim words.
    pub quote: String,
    /// The question their words answer, as the relaying agent stated it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub question: Option<String>,
    /// The agent that recorded the words for them: the event's `agent` source reference when it
    /// names someone other than the person themselves.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relayed_by: Option<String>,
    /// Where they said it (a session, thread or message), as the relaying agent stated it.
    /// Absent when nobody did.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub said_in: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accepted_at: Option<DateTime<Utc>>,
}

/// The statements recorded for `decision_id`, in the order they were recorded. One bulk read of
/// the (few) acceptance nodes: they have no edges, so the decision is matched on the node's
/// `decision_id` property.
pub(super) fn acceptance_statements(
    graph: &impl GraphView,
    decision_id: &str,
) -> Result<Vec<AcceptanceStatement>> {
    let mut statements: Vec<(i64, AcceptanceStatement)> = Vec::new();
    for (_, row) in node_rows(graph, NodeKind::Acceptance)? {
        if required_string(&row, "decision_id")? != decision_id {
            continue;
        }
        let accepted_by = optional_string(&row, "accepted_by").unwrap_or_default();
        let relayed_by = relayer_of(
            optional_string(&row, "source").as_deref(),
            optional_string(&row, "source_ref"),
            &accepted_by,
        );
        statements.push((
            optional_int(&row, "event_origin").unwrap_or(i64::MAX),
            AcceptanceStatement {
                accepted_by,
                quote: optional_string(&row, "quote").unwrap_or_default(),
                question: optional_string(&row, "question"),
                relayed_by,
                said_in: optional_string(&row, "said_in"),
                accepted_at: optional_datetime(&row, "accepted_at")?,
            },
        ));
    }
    statements.sort_by(|(a, left), (b, right)| {
        a.cmp(b)
            .then_with(|| left.accepted_by.cmp(&right.accepted_by))
    });
    Ok(statements
        .into_iter()
        .map(|(_, statement)| statement)
        .collect())
}
