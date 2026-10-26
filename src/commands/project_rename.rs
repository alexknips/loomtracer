//! Renaming a project (hivemind-x4xl7).
//!
//! A project handle is the name every decision filed under it carries, and the ledger never
//! rewrites an event, so a rename is not an edit of the handle. It is the facts that make the
//! new handle the old one's successor, each an ordinary event with its own reversal: register
//! the new handle, move the anchors and links across, declare the same topic keys, and move
//! each decision with a `decision.moved`. Nothing is dropped and nothing is orphaned: the old
//! handle stays registered with no decisions, anchors or links left, and the same call with the
//! two handles swapped puts everything back.
//!
//! The call can be repeated: a step already done is skipped, so a run that stopped half way
//! (the new handle's `part_of` parent differs from the old one's, say) is finished by running
//! it again once the cause is fixed.

use std::collections::BTreeMap;

use serde::Serialize;
use uuid::Uuid;

use crate::error::CommandError;
use crate::events::{DecisionMovedPayload, EventPayload, EventType};
use crate::ledger::EventLedger;
use crate::util::require_valid_actor_id;
use crate::Result;

use super::{
    classified_decisions_filed, payload_value_as_str, same_identifier, validate_project_handle,
    Commands,
};

/// What a rename names. A missing `display_name` or `purpose` is carried over from the old
/// project.
#[derive(Debug, Clone, Copy)]
pub struct ProjectRenameInput<'a> {
    pub actor_id: &'a str,
    pub from: &'a str,
    pub to: &'a str,
    pub display_name: Option<&'a str>,
    pub purpose: Option<&'a str>,
}

/// What a rename recorded. The one shape every surface serializes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProjectRenameOutcome {
    pub from: String,
    pub to: String,
    /// `false` when `to` was already registered, which a repeated run or a merge into an
    /// existing project does.
    pub registered: bool,
    pub anchors_moved: usize,
    pub links_moved: usize,
    pub topics_declared: usize,
    pub decisions_moved: usize,
}

impl<L: EventLedger> Commands<'_, L> {
    /// Make `input.to` the successor of `input.from`: the registration, anchors, links, topic
    /// vocabulary and decisions of `from` are carried to `to`. `from` must be registered; `to`
    /// must be a valid handle and may already be registered, in which case the two projects
    /// merge into it. Errors on the first fact that cannot be recorded (a rig anchor claimed
    /// by a third project, a `part_of` parent that differs), leaving what came before recorded.
    pub fn rename_project(&self, input: ProjectRenameInput<'_>) -> Result<ProjectRenameOutcome> {
        let ProjectRenameInput {
            actor_id,
            from,
            to,
            display_name,
            purpose,
        } = input;
        require_valid_actor_id(actor_id)?;
        validate_project_handle(to)?;
        if same_identifier(from, to) {
            return Err(CommandError::Validation("from and to must differ".to_owned()).into());
        }
        let Some(old) = self.find_project_registered(from)? else {
            return Err(CommandError::Invariant(format!(
                "project not registered: {from} -- nothing to rename"
            ))
            .into());
        };

        let registered = if self.project_exists(to)? {
            false
        } else {
            self.register_project(
                actor_id,
                to,
                display_name.or(old.display_name.as_deref()),
                purpose.or(old.purpose.as_deref()),
            )?;
            true
        };

        // The anchors go first: a rig anchor is unique, so `to` can hold it only once `from`
        // has let it go.
        let mut anchors_moved = 0;
        for (_, kind, value) in self
            .active_project_anchors()?
            .into_iter()
            .filter(|anchor| same_identifier(&anchor.0, from))
        {
            self.unanchor_project(actor_id, from, kind, &value)?;
            if !self.project_anchor_exists(to, kind, &value)? {
                self.anchor_project(actor_id, to, kind, &value)?;
            }
            anchors_moved += 1;
        }

        let mut links_moved = 0;
        for (link_from, link_to, kind) in self.active_project_links()? {
            let starts_at_old = same_identifier(&link_from, from);
            if !starts_at_old && !same_identifier(&link_to, from) {
                continue;
            }
            self.unlink_project(actor_id, &link_from, &link_to, kind)?;
            let (new_from, new_to) = if starts_at_old {
                (to, link_to.as_str())
            } else {
                (link_from.as_str(), to)
            };
            // A link between the two handles would link the new project to itself: it ends.
            if !same_identifier(new_from, new_to)
                && !self.project_link_exists(new_from, new_to, kind)?
            {
                self.link_project(actor_id, new_from, new_to, kind)?;
            }
            links_moved += 1;
        }

        let mut topics_declared = 0;
        for topic_key in self.project_topic_vocabulary(from)? {
            if self
                .declare_project_topic(actor_id, to, &topic_key)?
                .newly_declared
            {
                topics_declared += 1;
            }
        }

        // The decisions are found in one pass and moved without re-deriving each one's project:
        // `decisions_in_project` is the same filing `move_decision` checks `from` against.
        let reason = format!("project renamed from {from} to {to}");
        let mut decisions_moved = 0;
        for decision_id in self.decisions_in_project(from)? {
            let event = self.event_with_uuid(
                actor_id,
                EventPayload::DecisionMoved(DecisionMovedPayload {
                    decision_id,
                    from: from.to_owned(),
                    to: to.to_owned(),
                    reason: Some(reason.clone()),
                }),
                None,
                Uuid::new_v4(),
            )?;
            self.append_event(event)?;
            decisions_moved += 1;
        }

        Ok(ProjectRenameOutcome {
            from: from.to_owned(),
            to: to.to_owned(),
            registered,
            anchors_moved,
            links_moved,
            topics_declared,
            decisions_moved,
        })
    }

    /// The ids of the decisions now filed under `handle`, in id order: each decision's project
    /// is its proposal's, or the one its classification recorded (a decision a transcript was
    /// classified into), or the last `decision.moved` for it, in ledger order.
    fn decisions_in_project(&self, handle: &str) -> Result<Vec<String>> {
        let mut filed: BTreeMap<String, Option<String>> = BTreeMap::new();
        self.replay_types(
            &[
                EventType::DecisionProposed,
                EventType::DecisionMoved,
                EventType::IngestBatchClassified,
            ],
            |event| {
                if event.event_type == EventType::IngestBatchClassified {
                    for (decision_id, project, _) in classified_decisions_filed(event) {
                        filed.insert(decision_id, Some(project));
                    }
                    return Ok(());
                }
                let Some(decision_id) = payload_value_as_str(event, "decision_id") else {
                    return Ok(());
                };
                match event.event_type {
                    EventType::DecisionProposed => {
                        filed.insert(
                            decision_id.to_owned(),
                            payload_value_as_str(event, "project").map(str::to_owned),
                        );
                    }
                    EventType::DecisionMoved => {
                        if let Some(to) = payload_value_as_str(event, "to") {
                            filed.insert(decision_id.to_owned(), Some(to.to_owned()));
                        }
                    }
                    _ => {}
                }
                Ok(())
            },
        )?;
        Ok(filed
            .into_iter()
            .filter(|(_, project)| {
                project
                    .as_deref()
                    .is_some_and(|project| same_identifier(project, handle))
            })
            .map(|(decision_id, _)| decision_id)
            .collect())
    }
}
