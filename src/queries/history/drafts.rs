//! The decisions a classifier drafted from a transcript, as the history queries see them.
//!
//! A draft (`capture:<event>:<index>`) is one capture of an `ingest.batch_classified` event: no
//! `decision.proposed` records it, so the history queries, which walk the ledger's proposals,
//! never meet one. Its date, status and edges exist only in the graph, so they are read from it
//! and nowhere else: a draft is dated the way `GET /v1/graph` dates it (`decided_at`, the node's
//! `occurred_at`), which is the time of the turn it was said in, and its creation is the
//! classification event that recorded it. Nothing is inferred here and no draft is ranked.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use chrono::{DateTime, Utc};

use crate::commands::personal_project_handle;
use crate::events::{Event, EventId};
use crate::ledger::EventLedger;
use crate::projector::{GraphRow, GraphView, RelationKind};
use crate::queries::brief::is_classified_capture;
use crate::queries::shared::{
    optional_datetime, optional_int, optional_string, optional_string_list, query_error,
    relation_edges,
};
use crate::queries::status::DecisionStandings;
use crate::queries::DecisionStatus;
use crate::Result;

use super::{decision_event_provenance, DecisionEventProvenance};

/// One classifier-drafted decision, with what the history entries say of a decision. The decision
/// id is the key it is held under in `ClassifiedDrafts`.
#[derive(Debug)]
pub(super) struct ClassifiedDraft {
    /// When it was decided, as the graph dates it. `None` when the graph holds no time for it (a
    /// classification event with no timestamp and no turn time to fall back on).
    pub(super) decided_at: Option<DateTime<Utc>>,
    pub(super) title: String,
    pub(super) rationale: String,
    pub(super) quote: Option<String>,
    pub(super) question: Option<String>,
    pub(super) status: DecisionStatus,
    /// Address of the project it is filed under.
    pub(super) project: String,
    pub(super) topic_keys: Vec<String>,
    /// Who proposed, accepted or rejected it, and who recorded the classification.
    pub(super) actor_ids: Vec<String>,
    /// The classification event that recorded it.
    pub(super) creation: DecisionEventProvenance,
    pub(super) chosen_option_id: Option<String>,
    pub(super) option_ids: Vec<String>,
    pub(super) evidence_ids: Vec<String>,
    pub(super) hypothesis_ids: Vec<String>,
}

impl ClassifiedDraft {
    /// Whether it was decided after `after` and no later than `through`: the half-open window the
    /// event-based history queries cut a timestamp into. With no bound it is in the window; with
    /// one, a draft with no time is not.
    pub(super) fn decided_between(
        &self,
        after: Option<DateTime<Utc>>,
        through: Option<DateTime<Utc>>,
    ) -> bool {
        if after.is_none() && through.is_none() {
            return true;
        }
        let Some(decided_at) = self.decided_at else {
            return false;
        };
        after.is_none_or(|after| decided_at > after)
            && through.is_none_or(|through| decided_at <= through)
    }
}

/// Every draft of the graph, by decision id.
#[derive(Debug, Default)]
pub(super) struct ClassifiedDrafts {
    drafts: BTreeMap<String, ClassifiedDraft>,
}

impl ClassifiedDrafts {
    /// Reads the drafts out of `graph`. `decisions` is every decision node of the graph by id
    /// (`node_rows`), read once by the caller, who also needs the floor answers on it. `standings`
    /// is where every decision stands, read once by the caller too. Each draft's creation is the
    /// classification event its node names, which `ledger` is asked for by id: the caller has not
    /// read the classifications of the ledger, only the decisions. A draft that names an event the
    /// ledger does not hold is refused, not dropped: a window that quietly lost it would look
    /// complete.
    pub(super) fn load(
        graph: &impl GraphView,
        ledger: &impl EventLedger,
        decisions: &BTreeMap<String, GraphRow>,
        standings: &DecisionStandings,
    ) -> Result<Self> {
        let rows: Vec<(&String, &GraphRow)> = decisions
            .iter()
            .filter(|(id, _)| is_classified_capture(id))
            .collect();
        if rows.is_empty() {
            return Ok(Self::default());
        }
        let origins = rows
            .iter()
            .map(|(id, row)| {
                optional_int(row, "event_origin")
                    .and_then(|origin| EventId::try_from(origin).ok())
                    .ok_or_else(|| query_error(format!("draft {id} names no classification event")))
            })
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let classifications: HashMap<EventId, Event> = ledger
            .read_ids_for_tenant(&ledger.bound_tenant(), &origins)?
            .into_iter()
            .filter_map(|event| Some((event.event_id?, event)))
            .collect();

        let targets = |relation: RelationKind| -> Result<HashMap<String, Vec<String>>> {
            let mut by_source: HashMap<String, Vec<String>> = HashMap::new();
            for (from, to) in relation_edges(graph, relation)? {
                by_source.entry(from).or_default().push(to);
            }
            Ok(by_source)
        };
        let proposed_by = targets(RelationKind::ProposedBy)?;
        let accepted_by = targets(RelationKind::AcceptedBy)?;
        let rejected_by = targets(RelationKind::RejectedBy)?;
        let has_option = targets(RelationKind::HasOption)?;
        let chose = targets(RelationKind::Chose)?;
        let based_on = targets(RelationKind::BasedOn)?;
        let premised_on_direct = targets(RelationKind::PremisedOnDirect)?;
        let premised_on = targets(RelationKind::PremisedOn)?;
        let named = |by_source: &HashMap<String, Vec<String>>, id: &str| {
            by_source.get(id).cloned().unwrap_or_default()
        };

        let mut drafts = BTreeMap::new();
        for ((id, row), origin) in rows.into_iter().zip(origins) {
            let batch = classifications.get(&origin).ok_or_else(|| {
                query_error(format!(
                    "draft {id} was recorded by event {origin}, which the ledger does not hold"
                ))
            })?;
            let creation = decision_event_provenance(batch)?;

            // ubs:ignore: the draft owns its recorder twice, in its creation and in its actors
            let mut actor_ids = BTreeSet::from([creation.actor_id.clone()]);
            for by_source in [&proposed_by, &accepted_by, &rejected_by] {
                actor_ids.extend(named(by_source, id));
            }

            let option_ids = named(&has_option, id);
            let chosen_option_id = named(&chose, id).into_iter().next();
            let mut hypothesis_ids = named(&premised_on_direct, id);
            if let Some(chosen) = &chosen_option_id {
                hypothesis_ids.extend(named(&premised_on, chosen));
                hypothesis_ids.sort();
                hypothesis_ids.dedup();
            }
            let project = optional_string(row, "project")
                .unwrap_or_else(|| personal_project_handle(&creation.actor_id));

            let draft = ClassifiedDraft {
                decided_at: optional_datetime(row, "occurred_at")?,
                title: optional_string(row, "title").unwrap_or_default(),
                rationale: optional_string(row, "rationale").unwrap_or_default(),
                quote: optional_string(row, "quote"),
                question: optional_string(row, "question"),
                status: standings.status_of(id),
                project,
                topic_keys: optional_string_list(row, "topic_keys"),
                actor_ids: actor_ids.into_iter().collect(),
                creation,
                chosen_option_id,
                option_ids,
                evidence_ids: named(&based_on, id),
                hypothesis_ids,
            };
            drafts.insert(id.clone(), draft); // ubs:ignore: the map owns its ids; one clone per draft
        }
        Ok(Self { drafts })
    }

    /// Takes the draft out of the set, so the drafts left are the ones not yet met.
    pub(super) fn take(&mut self, decision_id: &str) -> Option<ClassifiedDraft> {
        self.drafts.remove(decision_id)
    }
}

/// The drafts left in the set, by decision id, in decision-id order.
impl IntoIterator for ClassifiedDrafts {
    type Item = (String, ClassifiedDraft);
    type IntoIter = std::collections::btree_map::IntoIter<String, ClassifiedDraft>;

    fn into_iter(self) -> Self::IntoIter {
        self.drafts.into_iter()
    }
}
