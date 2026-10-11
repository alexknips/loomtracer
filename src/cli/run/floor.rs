//! CLI side of the decision floor (hivemind-bbnw.10): the `--binds` / `--binds-nothing` and
//! `--undo-cost` / `--cheap-to-undo` flags of `emit decision.capture` and `supersede`, turned into
//! the two answers the commands record, and refused — with nothing written — when either is left
//! out.

use std::io::Write as IoWrite;

use crate::cli::args::{Cli, FloorArgs};
use crate::error::CliError;
use crate::floor::{BindsAnswer, FloorAnswers, FloorReply, UndoCostAnswer, BELOW_FLOOR_NOTE};
use crate::Result;

/// The refusal for a capture that leaves one of the two questions unanswered: both questions and
/// the flags that answer each, with the `nothing` / `cheap` answers spelled out.
pub(super) const FLOOR_REFUSAL: &str = "a captured decision must say what it binds and what undoing it would cost; nothing was written. Answer both: \
--binds <what it binds beyond the edit in front of you> or --binds-nothing, and \
--undo-cost <what undoing it would cost> or --cheap-to-undo";

fn sentence(flag: &'static str, value: &str) -> Result<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        Err(CliError::InvalidInput(format!("{flag} must not be empty")).into())
    } else {
        Ok(trimmed.to_owned())
    }
}

/// Collect the floor flags into the two answers. Exactly one of each pair is required (clap
/// already refuses both together), and a sentence must not be blank.
pub(super) fn floor_answers_from_args(args: &FloorArgs) -> Result<FloorAnswers> {
    let binds = match (&args.binds, args.binds_nothing) {
        (Some(text), false) => BindsAnswer::something(sentence("--binds", text)?),
        (None, true) => BindsAnswer::nothing(),
        _ => return Err(CliError::InvalidInput(FLOOR_REFUSAL.to_owned()).into()),
    };
    let undo_cost = match (&args.undo_cost, args.cheap_to_undo) {
        (Some(text), false) => UndoCostAnswer::costly(sentence("--undo-cost", text)?),
        (None, true) => UndoCostAnswer::cheap(),
        _ => return Err(CliError::InvalidInput(FLOOR_REFUSAL.to_owned()).into()),
    };
    Ok(FloorAnswers { binds, undo_cost })
}

/// Text mode only: say on `notices` (stderr in the CLI) that a capture is below the floor, so the
/// bare id on stdout stays what scripts parse. JSON replies carry `floor` and `below_floor`.
pub(super) fn announce_below_floor<W: IoWrite>(cli: &Cli, floor: &FloorReply, notices: &mut W) {
    if !cli.json && !floor.cleared {
        let _ = writeln!(notices, "below_floor: {BELOW_FLOOR_NOTE}");
    }
}
