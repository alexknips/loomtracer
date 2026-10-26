//! The project a classification files its decisions under (hivemind-zdsh.22).
//!
//! A capture client names the project of a session when it ships a batch (`ingest.batch_received`
//! carries `project` and `project_source`; `record_ingest_batch_for_project` checks the handle).
//! The classifier is never asked and cannot say: this module reads the batches a classification
//! covers and settles the project from what they state, and the write layer records the answer on
//! the `ingest.batch_classified` event, so the projector only copies it.
//!
//! The rule is the one a change that spans folders follows on the CLI (`--project-from-context`):
//!
//! - no covered batch states a project (a client that names none, a batch received before the
//!   field existed, a `loomtracer emit` capture): none, so the projector files each decision
//!   under the recorder's personal project, as it always did;
//! - the batches state one project: that project;
//! - they state several: the nearest project all of them are `part_of` (a project is the start
//!   of its own chain), else none. The reply names the projects either way.
//!
//! A batch that states no project says nothing and never votes: a session whose first batches
//! were shipped before its client was configured is still filed under the project the rest name.
//! How the project was determined is the source of the first covered batch that stated one.

use std::collections::{BTreeSet, HashMap, HashSet};

use crate::events::{EventId, EventType, ProjectLinkKind, ProjectSource};
use crate::ledger::EventLedger;
use crate::Result;

use super::{same_identifier, Commands};

/// How many received batches one lean read asks for, as in `require_received_batches`.
const RECEIVED_BATCH_PAGE: usize = 10_000;

/// Where a classification files its decisions, and the projects the covered batches named.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassificationFiling {
    /// The project and how it was determined; `None` when the decisions go to the personal
    /// project.
    pub project: Option<(String, ProjectSource)>,
    /// Every distinct project the covered batches stated, sorted. More than one means the
    /// classification spans projects.
    pub spanned: Vec<String>,
}

impl ClassificationFiling {
    /// The sentence a reply carries when the covered batches stated several projects, so a
    /// decision filed under a parent, or left in the personal project, is never a surprise.
    /// `None` when they stated one or none.
    pub fn span_notice(&self) -> Option<String> {
        if self.spanned.len() < 2 {
            return None;
        }
        let spanned = join_handles(&self.spanned);
        Some(match &self.project {
            Some((parent, _)) => format!("recorded for {parent}: these batches span {spanned}"),
            None => format!(
                "these batches span {spanned}, which share no parent: the decisions are saved to \
                 the recorder's personal project. Move them with loomtracer move ..., or register \
                 a parent."
            ),
        })
    }
}

/// `a`, `a and b`, `a, b and c`.
fn join_handles(handles: &[String]) -> String {
    match handles {
        [] => String::new(),
        [only] => only.clone(),
        [init @ .., last] => format!("{} and {last}", init.join(", ")),
    }
}

/// The nearest project every chain contains: each chain is a project followed by the projects
/// above it, nearest first. `None` for no chains and for chains that share nothing.
pub fn nearest_common_project(chains: &[Vec<&str>]) -> Option<String> {
    let (first, rest) = chains.split_first()?;
    first
        .iter()
        .find(|candidate| rest.iter().all(|chain| chain.contains(candidate)))
        .map(|candidate| (*candidate).to_owned())
}

impl<L: EventLedger> Commands<'_, L> {
    /// The filing of a classification covering `batch_ids`, from the projects those received
    /// batches state. `None` when none states one.
    pub(super) fn classification_filing(
        &self,
        batch_ids: &[String],
    ) -> Result<Option<ClassificationFiling>> {
        let named = self.projects_named_by_batches(batch_ids)?;
        let Some((_, first_source)) = named.first() else {
            return Ok(None);
        };
        let first_source = *first_source;
        let spanned: BTreeSet<&str> = named.iter().map(|(handle, _)| handle.as_str()).collect();

        let project = match spanned.len() {
            1 => named.first().map(|(handle, _)| handle.clone()),
            _ => self.nearest_parent_of(&spanned)?,
        };
        Ok(Some(ClassificationFiling {
            project: project.map(|handle| (handle, first_source)),
            spanned: spanned.into_iter().map(str::to_owned).collect(),
        }))
    }

    /// The project and source each covered batch stated, in the order the batches are named,
    /// one per batch. A batch received more than once keeps the first receipt that stated a
    /// project, like the session it names (a span sent again once the cell could record its
    /// project is filed under it); a batch that states none, or was never received, is left out.
    /// The read runs to the end of the received batches unless every covered batch has stated.
    fn projects_named_by_batches(
        &self,
        batch_ids: &[String],
    ) -> Result<Vec<(String, ProjectSource)>> {
        let wanted: HashSet<&str> = batch_ids.iter().map(String::as_str).collect();
        let mut stated: HashMap<String, (String, ProjectSource)> = HashMap::new();
        let mut offset: EventId = 0;
        while stated.len() < wanted.len() {
            let rows = self.ledger.read_fields_for_tenant(
                &self.context.tenant_id,
                EventType::IngestBatchReceived,
                &["batch_id", "project", "project_source"],
                offset,
                RECEIVED_BATCH_PAGE,
            )?;
            let next_offset = rows
                .last()
                .filter(|_| rows.len() == RECEIVED_BATCH_PAGE)
                .map(|last| last.event_id);
            for row in rows {
                let Ok([Some(batch_id), project, source]) =
                    <[Option<String>; 3]>::try_from(row.fields)
                else {
                    continue;
                };
                if !wanted.contains(batch_id.as_str()) || stated.contains_key(&batch_id) {
                    continue;
                }
                // A receipt that states no project says nothing: a later receipt of the same
                // batch may.
                let Some((handle, source)) =
                    project.zip(source.as_deref().and_then(ProjectSource::parse))
                else {
                    continue;
                };
                stated.insert(batch_id, (handle, source));
            }
            match next_offset {
                Some(next) => offset = next,
                None => break,
            }
        }

        // Taking each batch's entry out of `stated` also counts a batch named twice once.
        Ok(batch_ids
            .iter()
            .filter_map(|batch_id| stated.remove(batch_id.as_str()))
            .collect())
    }

    /// The nearest project every one of `handles` is part of, following the `part_of` links as
    /// they stand now. A project is the start of its own chain, so a classification covering a
    /// project's own batches and one of its sub-projects' is filed under that project. The write
    /// layer allows one parent per project but does not forbid a `part_of` cycle, so a walk stops
    /// before it revisits a project.
    fn nearest_parent_of(&self, handles: &BTreeSet<&str>) -> Result<Option<String>> {
        let links = self.active_project_links()?;
        let parent_of = |handle: &str| -> Option<&str> {
            links
                .iter()
                .find(|(from, _, kind)| {
                    *kind == ProjectLinkKind::PartOf && same_identifier(from, handle)
                })
                .map(|(_, to, _)| to.as_str())
        };
        let chains: Vec<Vec<&str>> = handles
            .iter()
            .map(|handle| {
                let mut chain = vec![*handle];
                let mut current = *handle;
                while let Some(parent) = parent_of(current) {
                    if chain.contains(&parent) {
                        break;
                    }
                    chain.push(parent);
                    current = parent;
                }
                chain
            })
            .collect();
        Ok(nearest_common_project(&chains))
    }
}

#[cfg(test)]
mod tests;
