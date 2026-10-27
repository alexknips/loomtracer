//! CLI side of `loomtracer reaffirm` and `loomtracer reexamine` (hivemind-ottw.4): say that a
//! decision still holds after something it rests on changed, and list what is waiting for that.
//!
//! `reaffirm` writes one `decision.reaffirmed`, attributed to whoever runs it, and only against an
//! open item (`crate::reaffirmation`): an ambiguous description, a decision with several open
//! items and no `--premise`, and every call with nothing to re-examine all happen before the
//! write. A refusal exits 2 with the reason. `reexamine` is a pure read.

use crate::cli::args::{Cli, ReaffirmArgs, ReexamineArgs};
use crate::cli::render::{
    format_query_response, format_reaffirm_ambiguous_output, format_reaffirm_output,
    render_reexamine_summary,
};
use crate::commands::Relay;
use crate::error::CliError;
use crate::projector::{memory::MemoryGraph, rebuild_graph_for_tenant};
use crate::queries::{get_reexamine_items, QueryContext, ReexamineRequest};
use crate::reaffirmation::{reaffirm, ReaffirmCall, ReaffirmOutcome};
use crate::Result;

use super::{
    cli_tenant, fluent_write_provenance, open_ledger, resolve_fluent_target, FluentResolution,
};

pub(super) fn run_reaffirm(cli: &Cli, args: &ReaffirmArgs) -> Result<String> {
    let tenant_id = cli_tenant(cli)?;
    let ledger = open_ledger(cli)?;

    let graph = MemoryGraph::default();
    rebuild_graph_for_tenant(&ledger, &tenant_id, &graph)?;
    let target = resolve_fluent_target(
        &cli.loomtracer_dir,
        !cli.json,
        &graph,
        args.decision_id.as_deref(),
        args.description.as_deref(),
        args.pick,
        args.topic.as_deref(),
    )?;
    let decision_id = match target {
        FluentResolution::Id(decision_id) => decision_id,
        FluentResolution::Output(output) => return Ok(output),
    };

    // `--quote` (clap requires `--reaffirmed-by` with it) is an agent recording a person's words;
    // otherwise `--because` (clap requires one of the two) is the actor's own.
    let call = match (&args.quote, &args.reaffirmed_by) {
        (Some(quote), Some(person)) => ReaffirmCall {
            actor_id: person.trim(),
            decision_id: &decision_id,
            premise: args.premise.as_deref(),
            reason: quote,
            relay: Some(Relay::new(&cli.actor, args.said_in.as_deref())),
        },
        _ => ReaffirmCall {
            actor_id: &cli.actor,
            decision_id: &decision_id,
            premise: args.premise.as_deref(),
            reason: args.because.as_deref().unwrap_or_default(),
            relay: None,
        },
    };
    let outcome = reaffirm(
        &ledger,
        &tenant_id,
        fluent_write_provenance(&cli.actor),
        &graph,
        call,
    )?;
    match outcome {
        ReaffirmOutcome::Recorded(reply) => format_reaffirm_output(cli.json, &reply),
        ReaffirmOutcome::Ambiguous(ambiguous) => {
            format_reaffirm_ambiguous_output(cli.json, &ambiguous)
        }
        ReaffirmOutcome::Refused(reason) => Err(CliError::InvalidInput(reason).into()),
    }
}

pub(super) fn run_reexamine(cli: &Cli, args: &ReexamineArgs) -> Result<String> {
    let tenant_id = cli_tenant(cli)?;
    let ledger = open_ledger(cli)?;

    let graph = MemoryGraph::default();
    rebuild_graph_for_tenant(&ledger, &tenant_id, &graph)?;
    // Narrowing to one decision resolves it like every fluent verb; with neither selector the
    // question is the whole tenant (or the project).
    let dependent_id = if args.decision.is_some() || args.decision_id.is_some() {
        let target = resolve_fluent_target(
            &cli.loomtracer_dir,
            !cli.json,
            &graph,
            args.decision_id.as_deref(),
            args.decision.as_deref(),
            args.pick,
            None,
        )?;
        match target {
            FluentResolution::Id(decision_id) => Some(decision_id),
            FluentResolution::Output(output) => return Ok(output),
        }
    } else {
        None
    };

    let request = ReexamineRequest {
        dependent_id,
        project: args.project.clone(),
        include_closed: args.all,
        limit: args.limit,
        cursor: args.cursor.clone(),
        ..ReexamineRequest::default()
    };
    let response = get_reexamine_items(&QueryContext::new(tenant_id), &graph, &ledger, &request)?;
    let next_cursor = response.data.next_cursor.clone();
    format_query_response(
        !cli.json,
        &response,
        |results| render_reexamine_summary(results, args.all),
        next_cursor.as_deref(),
    )
}
