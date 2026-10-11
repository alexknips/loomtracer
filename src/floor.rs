//! The decision floor, write side (hivemind-bbnw.10): every capture answers two short
//! questions, and the answers are recorded on the decision.
//!
//! 1. Does it bind anything beyond the edit in front of you (other people, other code, a later
//!    choice)? `nothing`, or `something` plus one sentence saying what.
//! 2. What would undoing it cost? `cheap`, or `costly` plus one sentence saying what.
//!
//! A choice clears the floor when it binds something OR is costly to undo. How much it binds and
//! how costly is not asked here: that stays a read-time ranking (Importance). The write path
//! cannot judge sentences (no model, no keyword guessing), so it reads the level the capturer
//! declared; the sentence is the record of why. Nothing is refused for being below the floor: a
//! below-floor capture lands and its reply says so (D1 = A1).
//!
//! This module holds the types, the one pure [`floor_verdict`] and the one note text, so the CLI,
//! MCP and HTTP verbs, the projector and the read side cannot drift. The wire form (`binds` /
//! `undo_cost` objects, shared by MCP and REST) is parsed once, by [`FloorAnswers::from_wire`],
//! the same way [`crate::grounding::GroundingSpec::from_wire`] parses `grounding`.

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

/// The two answers a CLI refusal, an MCP tool error and an HTTP 400 all name when a capture
/// leaves one out. Shared by every surface so they cannot drift.
pub const FLOOR_REFUSAL: &str = "a captured decision must say what it binds and what undoing it would cost; nothing was written. Answer both questions: binds (nothing, or something plus one sentence saying what) and undo cost (cheap, or costly plus one sentence saying what)";

/// Appended to the reply of a capture whose own answers put it below the floor. One sentence
/// pair, shared by every surface (CLI text, CLI JSON, MCP, HTTP).
pub const BELOW_FLOOR_NOTE: &str = "recorded, and marked below the decision floor: you said it binds nothing beyond the edit in front of you and is cheap to undo. An observation belongs in evidence and a request or task in your tracker. If this is a real choice and you left a leg out, supersede it with the answer that was missing";

/// What a decision binds beyond the edit in front of the capturer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BindsLevel {
    /// Nothing else depends on the choice.
    Nothing,
    /// Other people, other code or a later choice depend on it; the statement says what.
    Something,
}

impl BindsLevel {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Nothing => "nothing",
            Self::Something => "something",
        }
    }

    /// The level stored on a decision node, `None` for anything else.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "nothing" => Some(Self::Nothing),
            "something" => Some(Self::Something),
            _ => None,
        }
    }
}

/// What undoing a decision would cost.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UndoCostLevel {
    /// Reverting it is an ordinary edit.
    Cheap,
    /// Reverting it costs something real; the statement says what.
    Costly,
}

impl UndoCostLevel {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Cheap => "cheap",
            Self::Costly => "costly",
        }
    }

    /// The level stored on a decision node, `None` for anything else.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "cheap" => Some(Self::Cheap),
            "costly" => Some(Self::Costly),
            _ => None,
        }
    }
}

/// The answer to "what does it bind?", as recorded on `decision.proposed`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BindsAnswer {
    pub level: BindsLevel,
    /// One sentence saying what it binds. Present exactly when `level` is `something`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub statement: Option<String>,
}

/// The answer to "what would undoing it cost?", as recorded on `decision.proposed`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UndoCostAnswer {
    pub level: UndoCostLevel,
    /// One sentence saying what undoing it costs. Present exactly when `level` is `costly`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub statement: Option<String>,
}

impl BindsAnswer {
    /// `nothing`: no sentence.
    pub const fn nothing() -> Self {
        Self {
            level: BindsLevel::Nothing,
            statement: None,
        }
    }

    /// `something`, saying what.
    pub fn something(statement: impl Into<String>) -> Self {
        Self {
            level: BindsLevel::Something,
            statement: Some(statement.into()),
        }
    }

    /// The rule every surface and the event validator share: a sentence goes with `something`
    /// and only with `something`, and it is not blank.
    pub fn check(&self) -> std::result::Result<(), String> {
        check_statement(
            "binds",
            self.level == BindsLevel::Something,
            "something",
            "nothing",
            self.statement.as_deref(),
        )
    }
}

impl UndoCostAnswer {
    /// `cheap`: no sentence.
    pub const fn cheap() -> Self {
        Self {
            level: UndoCostLevel::Cheap,
            statement: None,
        }
    }

    /// `costly`, saying what.
    pub fn costly(statement: impl Into<String>) -> Self {
        Self {
            level: UndoCostLevel::Costly,
            statement: Some(statement.into()),
        }
    }

    /// See [`BindsAnswer::check`].
    pub fn check(&self) -> std::result::Result<(), String> {
        check_statement(
            "undo_cost",
            self.level == UndoCostLevel::Costly,
            "costly",
            "cheap",
            self.statement.as_deref(),
        )
    }
}

fn check_statement(
    field: &str,
    above_zero: bool,
    above_zero_level: &str,
    zero_level: &str,
    statement: Option<&str>,
) -> std::result::Result<(), String> {
    match (above_zero, statement) {
        (true, Some(text)) if !text.trim().is_empty() => Ok(()),
        (true, Some(_)) => Err(format!("`{field}.statement` must not be blank")),
        (true, None) => Err(format!(
            "`{field}` at level `{above_zero_level}` needs a `statement`: one sentence saying what"
        )),
        (false, Some(_)) => Err(format!(
            "`{field}` at level `{zero_level}` takes no `statement`; use level `{above_zero_level}` to say what"
        )),
        (false, None) => Ok(()),
    }
}

/// Both answers of one capture, owned. What a capture verb hands the commands layer: a call
/// that carries only one of the two never gets this far.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FloorAnswers {
    pub binds: BindsAnswer,
    pub undo_cost: UndoCostAnswer,
}

impl FloorAnswers {
    /// Parse the wire `binds` and `undo_cost` objects (MCP and REST). Both are required, each a
    /// `{level, statement?}` object with no other field; the refusal names both questions and
    /// their `nothing` / `cheap` answers whichever is missing.
    pub fn from_wire(
        binds: Option<&Value>,
        undo_cost: Option<&Value>,
    ) -> std::result::Result<Self, String> {
        let binds = binds
            .filter(|value| !value.is_null())
            .ok_or_else(|| FLOOR_REFUSAL.to_owned())?;
        let undo_cost = undo_cost
            .filter(|value| !value.is_null())
            .ok_or_else(|| FLOOR_REFUSAL.to_owned())?;
        let (level, statement) = wire_answer("binds", binds, ("nothing", "something"))?;
        let binds = BindsAnswer {
            level: BindsLevel::parse(&level).ok_or_else(|| not_a_level("binds", &level))?,
            statement,
        };
        let (level, statement) = wire_answer("undo_cost", undo_cost, ("cheap", "costly"))?;
        let undo_cost = UndoCostAnswer {
            level: UndoCostLevel::parse(&level).ok_or_else(|| not_a_level("undo_cost", &level))?,
            statement,
        };
        binds.check()?;
        undo_cost.check()?;
        Ok(Self { binds, undo_cost })
    }

    /// The verdict these answers reach.
    pub fn verdict(&self) -> FloorVerdict {
        floor_verdict(Some(self.binds.level), Some(self.undo_cost.level))
    }

    /// The `floor` object of a capture reply, and the note when the answers are below it.
    pub fn reply(&self) -> FloorReply {
        FloorReply {
            binds: self.binds.level,
            undo_cost: self.undo_cost.level,
            cleared: self.verdict() == FloorVerdict::Cleared,
        }
    }
}

fn not_a_level(field: &str, level: &str) -> String {
    format!("`{field}.level` `{level}` is not an accepted level")
}

/// One `{level, statement?}` object: the level string (one of `levels`) and the statement.
fn wire_answer(
    field: &str,
    value: &Value,
    levels: (&str, &str),
) -> std::result::Result<(String, Option<String>), String> {
    let (zero, above) = levels;
    let Value::Object(map) = value else {
        return Err(format!(
            "`{field}` must be an object {{level: {zero} | {above}, statement?}}"
        ));
    };
    if let Some(unknown) = map
        .keys()
        .find(|key| key.as_str() != "level" && key.as_str() != "statement")
    {
        return Err(format!("`{field}` has unknown field `{unknown}`"));
    }
    let level = match map.get("level") {
        Some(Value::String(level)) if level == zero || level == above => level.clone(),
        Some(Value::String(level)) => {
            return Err(format!(
                "`{field}.level` `{level}` is not one of {zero}, {above}"
            ))
        }
        _ => return Err(format!("`{field}.level` must be one of {zero}, {above}")),
    };
    Ok((level, wire_statement(map, field)?))
}

fn wire_statement(
    map: &Map<String, Value>,
    field: &str,
) -> std::result::Result<Option<String>, String> {
    match map.get("statement") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => Ok(Some(text.clone())),
        Some(_) => Err(format!("`{field}.statement` must be a string")),
    }
}

/// Where a decision stands against the floor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FloorVerdict {
    /// Binds something or is costly to undo.
    Cleared,
    /// Binds nothing and is cheap to undo, by the capturer's own answers.
    Below,
    /// Never asked: an event from before the answers existed, a raw emit, a classifier
    /// ingest, an import, Slack. Not a verdict on the decision.
    NotAsked,
}

impl FloorVerdict {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Cleared => "cleared",
            Self::Below => "below",
            Self::NotAsked => "not_asked",
        }
    }
}

/// The floor, as a pure function of the two recorded levels: cleared when it binds something or
/// is costly to undo, below when it does neither, and not asked unless BOTH answers exist. No
/// inference and no model: the same two levels always give the same verdict.
pub fn floor_verdict(binds: Option<BindsLevel>, undo_cost: Option<UndoCostLevel>) -> FloorVerdict {
    match (binds, undo_cost) {
        (Some(binds), Some(undo_cost)) => {
            if binds == BindsLevel::Something || undo_cost == UndoCostLevel::Costly {
                FloorVerdict::Cleared
            } else {
                FloorVerdict::Below
            }
        }
        _ => FloorVerdict::NotAsked,
    }
}

/// [`floor_verdict`] from the level strings stored on a decision node (`binds_level`,
/// `undo_cost_level`); an absent or unrecognized string reads as not asked.
pub fn floor_verdict_from_stored(
    binds_level: Option<&str>,
    undo_cost_level: Option<&str>,
) -> FloorVerdict {
    floor_verdict(
        binds_level.and_then(BindsLevel::parse),
        undo_cost_level.and_then(UndoCostLevel::parse),
    )
}

/// The `floor` object every capture reply carries: both levels and whether the floor was cleared.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct FloorReply {
    pub binds: BindsLevel,
    pub undo_cost: UndoCostLevel,
    pub cleared: bool,
}

impl FloorReply {
    /// `Some(true)` when the answers are below the floor, so a reply names it only then.
    pub const fn below_floor(&self) -> Option<bool> {
        if self.cleared {
            None
        } else {
            Some(true)
        }
    }

    /// The note that goes with a below-floor reply.
    pub const fn note(&self) -> Option<&'static str> {
        if self.cleared {
            None
        } else {
            Some(BELOW_FLOOR_NOTE)
        }
    }

    /// Put `floor` into a JSON reply, and `below_floor: true` with the note when the answers
    /// are below the floor. A cleared capture adds nothing else.
    pub fn insert_into(&self, reply: &mut Value) {
        if let Some(reply) = reply.as_object_mut() {
            reply.insert("floor".to_owned(), json!(self));
            if !self.cleared {
                reply.insert("below_floor".to_owned(), json!(true));
                reply.insert("below_floor_note".to_owned(), json!(BELOW_FLOOR_NOTE));
            }
        }
    }
}

#[cfg(test)]
mod tests;
