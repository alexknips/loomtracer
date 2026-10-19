mod ask;
#[cfg(test)]
mod beside_cell_tests;
mod floor;
mod ground;
mod grounding;
mod migrate;
mod reaffirm;

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::Write as _;
use std::io::{self, BufRead, Read as IoRead, Write as IoWrite};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

use crate::commands::{
    CaptureWarning, CommandContext, Commands, DecisionPlacement, DecisionProposalInput,
    DeterminedProject, FloorInput, Grounding, SupersedeInput,
};
use crate::error::{CliError, CommandError, LedgerError};
use crate::events::{
    CaptureItem, Event, EventPayload, EventProvenance, HypothesisKind, ProjectSource,
    RelationKind as EventRelationKind, TenantId,
};
use crate::identity::{
    agent_actor_id, agent_present_in_env, default_agent_session, default_agent_tool,
    default_human_actor_id, raw_agent_session_from_env,
};
use crate::importance;
use crate::ingest::{
    accumulate_file_summary_pub, extract_slack_decision_draft, import_documents,
    import_prose_file_candidates, import_slack_thread, parse_slack_thread_fixture,
    prepare_document_texts, DocumentFileImportStatus, DocumentImportFormat, DocumentImportRequest,
    DocumentImportSummary, DocumentPreparationRequest, ProseImportCandidate, ProseImportSource,
    SlackIngestOutcome,
};
use crate::ledger::{
    AnyLedger, EventLedger, LedgerConfig, SqliteEventLedger, TenantScopedLedger,
    TenantScopedOwnedLedger,
};
use crate::projector::{memory::MemoryGraph, rebuild_graph_for_tenant, GraphView};
use crate::quality_profile::{
    self, decision_log_section, parse_kinds, ScanRequest, SuggestionsRequest,
};
use crate::queries::{
    decisions_in_project, derive_decision_status, export_decision_log, export_read_only_summary,
    get_active_decision_blockers, get_blocker_notification_candidates, get_changed_decisions,
    get_compact_view, get_contested_decisions, get_decision, get_decision_brief,
    get_decision_neighborhood, get_decision_timeline, get_decisions_added_since,
    get_decisions_changed_since, get_open_why_decisions, get_project, get_recent_activity,
    get_recent_decisions, get_relevant_decisions, get_situational_decisions,
    get_supersession_chain, get_waiting_requests, list_projects, misfiled_next_cursor,
    require_registered_project, resolve_decision_by_description, resolve_decision_for_reading,
    scan_misfiled_decisions, search_decisions, search_decisions_any, upstream_of,
    ActiveDecisionBlockersRequest, BlockerNotificationCandidatesRequest, ChangedDecisionsRequest,
    ChangedSinceRequest, ContestedDecisionsRequest, DecisionBlockerFilters, DecisionLogExport,
    DecisionLogOutcome, DecisionLogRequest, DecisionStatus, DecisionsAddedSinceFilterRequest,
    DecisionsAddedSinceRequest, HistoryFilterRequest, MisfiledScanRequest, NeighborhoodRequest,
    OpenWhyRequest, OptionLabel, ProjectDecisionsRequest, ProjectListRequest, ProjectOutcome,
    QueryContext, ReadOnlyExportQuery, ReadOnlyExportRequest, RecentActivityRequest,
    RecentDecisionEntry, RecentDecisionFilterRequest, RecentDecisionsRequest, ResolveOutcome,
    ResolvedCandidate, SearchDecisionRequest, SituationalRequest, WaitingRequestsRequest,
};
use crate::slack_app::{
    handle_slack_command, slack_app_manifest, slack_oauth_install_url, SlackAppStore,
    SlackCaptureRequest, SlackCommandRequest, SlackWorkspaceInstall,
};
use crate::suggest::{
    propose_document_extraction_candidates, DocumentCandidateExtractor, DocumentCandidateRequest,
    DocumentExtractionSourceDocument,
};
use crate::summarize::{
    recall_decisions, weekly_digest, DigestRequest, RecallRequest, RECALL_MAX_LIMIT,
};
use crate::{HivemindError, Result};

use super::args::{
    ClassifyQueueArgs, ClassifyQueueCommand, ClassifyQueueListArgs, ClassifyQueueSubmitArgs, Cli,
    Command, ConnectorArgs, ConnectorAuthArgs, ConnectorCommand, DecisionCaptureSource, DigestArgs,
    DisagreeArgs, DumpArgs, DumpFormat, EmitArgs, EmitCaptureProvenanceArgs, EmitCommand,
    EmitDecisionProposedArgs, EmitHypothesisKind, EmitRelationKind, ExportArgs, GraphBackend,
    ImportArgs, ImportCommand, ImportConnectorCommand, ImportDocumentsArgs, IngestArgs,
    IngestCommand, IngestSlackThreadArgs, MapArgs, McpArgs, MoveArgs, OptionLabelArgs,
    ProjectAnchorArgs, ProjectArgs, ProjectCommand, ProjectDecisionsArgs, ProjectDeclareTopicArgs,
    ProjectLinkArgs, ProjectListArgs, ProjectRegisterArgs, ProjectShowArgs, ProjectSourceArg,
    ProjectUseArgs, QualityScanArgs, QueryAddedSinceArgs, QueryArgs, QueryBlockerPriority,
    QueryChangedDecisionsArgs, QueryChangedSinceArgs, QueryCommand, QueryDecisionStatus,
    QueryExportKind, QueryExportReadOnlySummaryArgs, QueryHistoryFilterArgs,
    QueryRankDecisionsByImportanceArgs, QueryRecentActivityArgs, QueryRecentDecisionsArgs,
    QueryRelationKind, QueryScanDecisionQualityArgs, QuerySearchDecisionsArgs,
    QuerySituationalArgs, QuickstartArgs, RestatementsArgs, RestatementsCommand, RetitleArgs,
    ReviewArgs, ServeArgs, SlackAppArgs, SlackAppCommand, SupersedeArgs, TenantArgs, TenantCommand,
    TenantCreateArgs, TuiArgs,
};
use super::current_project::CurrentProjectStore;
use super::project_context::{resolve_project_in_ledger, ProjectContextEnv, ResolvedProject};
use super::render::{
    append_truncation_notice, decision_status_label, format_capture_output,
    format_close_matched_response, format_current_project_output, format_disagree_output,
    format_export_output, format_import_output, format_json_value, format_move_output,
    format_output, format_prepare_documents_output, format_project_anchor_output,
    format_project_decisions_output, format_project_declare_topic_output,
    format_project_link_output, format_project_list_output, format_project_register_output,
    format_project_show_output, format_query_response, format_retitle_output, format_review_output,
    format_supersede_output, render_active_blockers_summary, render_added_since_summary,
    render_applied_links, render_blocker_notifications_summary, render_changed_decisions_summary,
    render_changed_since_summary, render_compact_view_summary, render_contested_decisions_summary,
    render_decision_brief_summary, render_decision_list_summary, render_decision_summary,
    render_dot, render_importance_summary, render_misfiled_scan_summary,
    render_neighborhood_summary, render_open_why_summary, render_placement_line,
    render_read_only_export_summary, render_recall_summary, render_recent_activity_summary,
    render_recent_decisions_summary, render_resolve_outcome_summary, render_restatement_proposals,
    render_scan_report_summary, render_score_report_summary, render_search_summary,
    render_situational_summary, render_supersession_summary, render_waiting_requests_summary,
    CaptureCommandOutput, CurrentProjectOutput, DisagreeCommandOutput, ExportReport,
    OutputEnvelope, ProjectAnchorOutput, ProjectDeclareTopicOutput, ProjectLinkOutput,
    ProjectRegisterOutput, ReviewActionOutput, ReviewCommandOutput, SupersedeCommandOutput,
};

pub fn run(cli: &Cli) -> Result<String> {
    validate_global_flags(cli)?;

    let beside_cell = LocalLedgerBesideCell::from_cli(cli);
    match &beside_cell {
        Some(cell) => cell.refuse_without_local_ledger()?,
        None => refuse_verb_without_ledger(cli)?,
    }
    let output = dispatch(cli)?;
    with_read_notices(cli, output, beside_cell.as_ref(), &mut io::stderr())
}

/// The read verbs answer even when the ledger holds annotation rows nobody can parse (a
/// malformed assessment): the projector and every history read skip them. What they do not do is
/// skip them silently: the answer carries a one-line notice naming the rows. A verb run with
/// `HIVEMIND_API_URL` set also says that it used the local ledger, not the server (see
/// [`LocalLedgerBesideCell`]); where stdout is data, `emit`'s bare id and `dump`'s DOT, that
/// notice goes to stderr instead.
fn with_read_notices<W: IoWrite>(
    cli: &Cli,
    output: String,
    cell: Option<&LocalLedgerBesideCell>,
    stderr: &mut W,
) -> Result<String> {
    let reads_annotations = matches!(
        cli.command,
        Command::Query(_)
            | Command::Export(_)
            | Command::Digest(_)
            | Command::QualityScan(_)
            | Command::Reexamine(_)
    );
    if cell.is_none() && !reads_annotations {
        return Ok(output);
    }
    let mut notices = Vec::new();
    if let Some(cell) = cell {
        let notice = cell.used_locally_notice();
        if notice_goes_to_stderr(cell.verb, &output) {
            // Best effort: the command has already run, so a closed stderr must not fail it.
            let _ = writeln!(stderr, "notice: {notice}");
        } else {
            notices.push(notice);
        }
    }
    if reads_annotations {
        let tenant_id = cli_tenant(cli)?;
        let ledger = open_ledger(cli)?;
        let rows = crate::read_notice::unreadable_annotations(&ledger, &tenant_id)?;
        notices.extend(crate::read_notice::notice_for(&rows));
    }
    Ok(if notices.is_empty() {
        output
    } else {
        crate::read_notice::annotate_output(output, &notices.join(" | "))
    })
}

/// A JSON object carries the notice as a `notice` key and text as a final `notice:` line. What is
/// neither (a bare id, DOT, a JSON array such as `map --alpha` twice prints) would be broken by
/// either, so its notice goes to stderr.
fn notice_goes_to_stderr(verb: CellBesideVerb, output: &str) -> bool {
    match serde_json::from_str::<serde_json::Value>(output) {
        Ok(value) => !value.is_object(),
        Err(_) => verb.stdout_is_data(),
    }
}

/// The verbs that open the ledger under `--hivemind-dir`. All but `classify-queue` are not told
/// about the server in `HIVEMIND_API_URL`; `classify-queue` honours it and opens no ledger while it
/// is set (see [`CellBesideVerb::honours_api_url`]).
#[derive(Debug, Clone, Copy)]
enum CellBesideVerb {
    Query,
    Digest,
    Export,
    Dump,
    Map,
    RestatementsPropose,
    Review,
    Emit,
    Disagree,
    Supersede,
    Move,
    Retitle,
    Ground,
    Reaffirm,
    Reexamine,
    Ask,
    Import,
    RestatementsApply,
    QualityScan,
    ProjectRegister,
    ProjectLink,
    ProjectUnlink,
    ProjectAnchor,
    ProjectDeclareTopic,
    ProjectList,
    ProjectShow,
    ProjectDecisions,
    ProjectUse,
    IngestSlackThread,
    ClassifyQueueList,
    ClassifyQueueSubmit,
    Tui,
    TenantCreate,
}

/// What a verb does with the ledger it opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LedgerUse {
    /// Answers from the ledger and records nothing.
    Reads,
    /// Reads the ledger and may write to it: a verb that acts on what the ledger already holds,
    /// and so has nothing to act on in a folder that holds none.
    ReadsAndWrites,
    /// Records into the ledger, and so may start one.
    WritesTo,
}

impl LedgerUse {
    fn phrase(self) -> &'static str {
        match self {
            Self::Reads => "reads",
            Self::ReadsAndWrites => "reads and writes",
            Self::WritesTo => "writes to",
        }
    }
}

impl CellBesideVerb {
    /// The verb `command` is, when it opens the ledger under `--hivemind-dir`. Every verb and
    /// subcommand is named, with no catch-all arm, so adding one fails to compile here until
    /// someone decides whether it needs the guard; a verb left out says why.
    fn of(command: &Command) -> Option<Self> {
        Some(match command {
            // Makes its own throwaway ledger on purpose and prints its path.
            Command::Quickstart(_) => return None,
            Command::Emit(_) => Self::Emit,
            Command::Disagree(_) => Self::Disagree,
            Command::Supersede(_) => Self::Supersede,
            Command::Move(_) => Self::Move,
            Command::Retitle(_) => Self::Retitle,
            Command::Ground(_) => Self::Ground,
            Command::Reaffirm(_) => Self::Reaffirm,
            Command::Reexamine(_) => Self::Reexamine,
            Command::Ask(_) => Self::Ask,
            Command::Review(_) => Self::Review,
            Command::Import(args) => match args.command {
                ImportCommand::Documents(_) | ImportCommand::Connector(_) => Self::Import,
                // Turns files into text files and opens no ledger.
                ImportCommand::PrepareDocuments(_) => return None,
            },
            Command::Query(_) => Self::Query,
            Command::Dump(_) => Self::Dump,
            Command::Tui(_) => Self::Tui,
            Command::Ingest(args) => match args.command {
                IngestCommand::SlackThread(_) => Self::IngestSlackThread,
            },
            // The Slack app's pieces over a local dir: ignoring HIVEMIND_API_URL is the standing
            // rmdi1 ruling, as for `mcp`.
            Command::SlackApp(args) => match args.command {
                SlackAppCommand::Manifest(_)
                | SlackAppCommand::OauthUrl(_)
                | SlackAppCommand::Install(_)
                | SlackAppCommand::EnqueueCapture(_)
                | SlackAppCommand::Drain(_)
                | SlackAppCommand::Command(_) => return None,
            },
            // Servers over a local dir: the standing rmdi1 ruling is that they ignore the variable.
            Command::Mcp(_) | Command::Serve(_) => return None,
            // Reads the explicit source dir it is given and writes to the destination it names.
            // It refuses a source with no ledger itself, before it opens one.
            Command::Migrate(_) => return None,
            Command::Map(_) => Self::Map,
            Command::Digest(_) => Self::Digest,
            // Honours HIVEMIND_API_URL: with it set, it talks to the server instead of opening a
            // ledger. With it unset, it opens the one under `--hivemind-dir`.
            Command::ClassifyQueue(args) => match args.command {
                ClassifyQueueCommand::List(_) => Self::ClassifyQueueList,
                ClassifyQueueCommand::Submit(_) => Self::ClassifyQueueSubmit,
            },
            Command::Restatements(args) => match args.command {
                RestatementsCommand::Propose(_) => Self::RestatementsPropose,
                RestatementsCommand::Apply(_) => Self::RestatementsApply,
            },
            // Keeps only an OAuth token file under the hivemind dir; no ledger.
            Command::Connector(args) => match args.command {
                ConnectorCommand::Auth(_) => return None,
            },
            Command::QualityScan(_) => Self::QualityScan,
            Command::Export(_) => Self::Export,
            Command::Tenant(args) => match args.command {
                TenantCommand::Create(_) => Self::TenantCreate,
            },
            Command::Project(args) => match &args.command {
                ProjectCommand::Register(_) => Self::ProjectRegister,
                ProjectCommand::Link(_) => Self::ProjectLink,
                ProjectCommand::Unlink(_) => Self::ProjectUnlink,
                ProjectCommand::Anchor(_) => Self::ProjectAnchor,
                ProjectCommand::DeclareTopic(_) => Self::ProjectDeclareTopic,
                ProjectCommand::List(_) => Self::ProjectList,
                // `--current` reads only the current-project file.
                ProjectCommand::Show(show) if show.current => return None,
                ProjectCommand::Show(_) => Self::ProjectShow,
                ProjectCommand::Decisions(_) => Self::ProjectDecisions,
                // `--clear` touches only the current-project file; setting a handle first checks
                // it against the project registry in the ledger.
                ProjectCommand::Use(set) if set.handle.is_none() || set.clear => return None,
                ProjectCommand::Use(_) => Self::ProjectUse,
            },
        })
    }

    fn name(self) -> &'static str {
        match self {
            Self::Query => "query",
            Self::Digest => "digest",
            Self::Export => "export",
            Self::Dump => "dump",
            Self::Map => "map",
            Self::RestatementsPropose => "restatements propose",
            Self::Review => "review",
            Self::Emit => "emit",
            Self::Disagree => "disagree",
            Self::Supersede => "supersede",
            Self::Move => "move",
            Self::Retitle => "retitle",
            Self::Ground => "ground",
            Self::Reaffirm => "reaffirm",
            Self::Reexamine => "reexamine",
            Self::Ask => "ask",
            Self::Import => "import",
            Self::RestatementsApply => "restatements apply",
            Self::QualityScan => "quality-scan",
            Self::ProjectRegister => "project register",
            Self::ProjectLink => "project link",
            Self::ProjectUnlink => "project unlink",
            Self::ProjectAnchor => "project anchor",
            Self::ProjectDeclareTopic => "project declare-topic",
            Self::ProjectList => "project list",
            Self::ProjectShow => "project show",
            Self::ProjectDecisions => "project decisions",
            Self::ProjectUse => "project use",
            Self::IngestSlackThread => "ingest slack-thread",
            Self::ClassifyQueueList => "classify-queue list",
            Self::ClassifyQueueSubmit => "classify-queue submit",
            Self::Tui => "tui",
            Self::TenantCreate => "tenant create",
        }
    }

    /// The verb talks to the server in `HIVEMIND_API_URL` when that is set, and opens a local
    /// ledger only when it is not. The guard for a verb that ignores the variable (see
    /// [`LocalLedgerBesideCell`]) therefore does not apply to it, and its refusal for a folder
    /// with no ledger applies only while the variable is unset.
    fn honours_api_url(self) -> bool {
        matches!(self, Self::ClassifyQueueList | Self::ClassifyQueueSubmit)
    }

    /// What the verb does to the local ledger. Every verb is placed with no catch-all arm, so a
    /// new one fails to compile until someone decides which it is. A `Reads` or `ReadsAndWrites`
    /// verb is refused a folder that holds no ledger; only a `WritesTo` verb may start one (see
    /// [`refuse_verb_without_ledger`]).
    fn ledger_use(self) -> LedgerUse {
        match self {
            Self::Query
            | Self::Digest
            | Self::Export
            | Self::Dump
            | Self::Map
            | Self::RestatementsPropose
            | Self::ProjectList
            | Self::ProjectShow
            | Self::ProjectDecisions
            | Self::Reexamine
            | Self::ClassifyQueueList
            | Self::Tui => LedgerUse::Reads,
            Self::Review | Self::QualityScan => LedgerUse::ReadsAndWrites,
            Self::Emit
            | Self::Ask
            | Self::Import
            | Self::ProjectRegister
            | Self::IngestSlackThread
            | Self::TenantCreate => LedgerUse::WritesTo,
            // These find the decision they act on in the ledger, then write to it. `classify-queue
            // submit` finds the batches it classifies the same way.
            Self::ClassifyQueueSubmit
            | Self::Disagree
            | Self::Supersede
            | Self::Move
            | Self::Retitle
            | Self::Ground
            | Self::Reaffirm
            | Self::RestatementsApply
            // These act on projects the registry already holds, and `project use` checks its
            // handle against the registry before it stores the setting.
            | Self::ProjectLink
            | Self::ProjectUnlink
            | Self::ProjectAnchor
            | Self::ProjectDeclareTopic
            | Self::ProjectUse => LedgerUse::ReadsAndWrites,
        }
    }

    /// What the verb does to the local ledger, for "`hivemind X` ___ a local ledger".
    fn uses(self) -> &'static str {
        self.ledger_use().phrase()
    }

    fn records(self) -> bool {
        !matches!(
            self,
            Self::Query
                | Self::Digest
                | Self::Export
                | Self::Dump
                | Self::Map
                | Self::RestatementsPropose
                | Self::ProjectList
                | Self::ProjectShow
                | Self::ProjectDecisions
                | Self::ProjectUse
                | Self::Reexamine
                | Self::ClassifyQueueList
                | Self::Tui
        )
    }

    /// `emit` prints the new id and `dump` prints DOT: a line added to stdout would break the
    /// script or `dot` that reads it. `tui` owns the terminal while it runs, so its notice goes to
    /// stderr once it has exited.
    fn stdout_is_data(self) -> bool {
        matches!(self, Self::Emit | Self::Dump | Self::Tui)
    }
}

/// A verb run with `HIVEMIND_API_URL` set, which it never contacts. A person who follows the
/// cell's docs (`docs/KEYLESS_CAPTURE.md`) has that variable set, and without this their `query`
/// answered "0 decisions" from an empty ledger the command made up in the current folder (or from
/// whatever ledger the folder held), and `emit` recorded the decision there and exited 0, with
/// nothing to say it was not the cell's. So the verb refuses when there is no local ledger (and
/// makes none), and otherwise names the ledger it used. A configured shared database is a direct
/// read of the cell's own data, so it is left alone.
struct LocalLedgerBesideCell {
    verb: CellBesideVerb,
    api_url: String,
    hivemind_dir: PathBuf,
}

impl LocalLedgerBesideCell {
    fn from_cli(cli: &Cli) -> Option<Self> {
        // A verb that honours the variable talks to that server, so it is no verb beside a cell.
        let verb = CellBesideVerb::of(&cli.command).filter(|verb| !verb.honours_api_url())?;
        let config = LedgerConfig::from_cli(cli);
        if config
            .database_url
            .as_deref()
            .is_some_and(|url| !url.is_empty())
        {
            return None;
        }
        Some(Self {
            verb,
            api_url: hivemind_api_url_from_env()?,
            hivemind_dir: config.hivemind_dir,
        })
    }

    fn refuse_without_local_ledger(&self) -> Result<()> {
        if SqliteEventLedger::exists_in(&self.hivemind_dir) {
            return Ok(());
        }
        let dir = self.hivemind_dir.display();
        let nothing_recorded = if self.verb.records() {
            ", and nothing was recorded"
        } else {
            ""
        };
        let local_instead = match self.verb {
            CellBesideVerb::Emit
            | CellBesideVerb::Ask
            | CellBesideVerb::Import
            | CellBesideVerb::ProjectRegister
            | CellBesideVerb::IngestSlackThread
            | CellBesideVerb::TenantCreate => format!(
                "To record to a local ledger instead, unset HIVEMIND_API_URL (the ledger is then \
                 created under {dir}) or point --hivemind-dir at an existing one"
            ),
            CellBesideVerb::Review => "To review a local ledger, unset HIVEMIND_API_URL and point \
                 --hivemind-dir at it"
                .to_owned(),
            // These act on decisions a ledger already holds, so a ledger made empty would not help.
            CellBesideVerb::Disagree
            | CellBesideVerb::Supersede
            | CellBesideVerb::Move
            | CellBesideVerb::Retitle
            | CellBesideVerb::Ground
            | CellBesideVerb::Reaffirm
            | CellBesideVerb::RestatementsApply
            | CellBesideVerb::QualityScan
            | CellBesideVerb::ProjectLink
            | CellBesideVerb::ProjectUnlink
            | CellBesideVerb::ProjectAnchor
            | CellBesideVerb::ProjectDeclareTopic
            | CellBesideVerb::ClassifyQueueSubmit
            | CellBesideVerb::ProjectUse => {
                "To use a local ledger, unset HIVEMIND_API_URL and point --hivemind-dir at it"
                    .to_owned()
            }
            CellBesideVerb::Query
            | CellBesideVerb::Digest
            | CellBesideVerb::Export
            | CellBesideVerb::Dump
            | CellBesideVerb::Map
            | CellBesideVerb::RestatementsPropose
            | CellBesideVerb::ProjectList
            | CellBesideVerb::ProjectShow
            | CellBesideVerb::ProjectDecisions
            | CellBesideVerb::Reexamine
            | CellBesideVerb::ClassifyQueueList
            | CellBesideVerb::Tui => {
                "To read a local ledger, unset HIVEMIND_API_URL and point --hivemind-dir at it"
                    .to_owned()
            }
        };
        Err(CliError::InvalidInput(format!(
            "HIVEMIND_API_URL is set ({url}), but `hivemind {verb}` {uses} a local ledger and \
             never contacts that server, and there is no local ledger under {dir}; none was \
             created{nothing_recorded}. {hint}. {local_instead}",
            url = self.api_url,
            verb = self.verb.name(),
            uses = self.verb.uses(),
            hint = self.cell_route_hint(),
        ))
        .into())
    }

    fn used_locally_notice(&self) -> String {
        let dir = self.hivemind_dir.display();
        let verb = self.verb.name();
        let what = match self.verb {
            CellBesideVerb::Emit => format!(
                "this was recorded in the local ledger under {dir}: `hivemind emit` does not \
                 write to that server, so the cell does not have it"
            ),
            // A verb that can find nothing to act on, or ends in a dry run or a list of
            // candidates, writes nothing: say where a write would have gone, not that one did.
            _ if self.verb.records() => format!(
                "this used the local ledger under {dir}, and anything it recorded went there: \
                 `hivemind {verb}` does not use that server, so the cell does not have it"
            ),
            _ => format!(
                "this answer is from the local ledger under {dir}: `hivemind {verb}` does not \
                 read that server"
            ),
        };
        format!(
            "HIVEMIND_API_URL is set ({url}), but {what}. {hint}",
            url = self.api_url,
            hint = self.cell_route_hint(),
        )
    }

    /// The cell's own HTTP routes that do what the verb does against a local ledger.
    fn cell_route_hint(&self) -> String {
        let url = &self.api_url;
        match self.verb {
            CellBesideVerb::Emit => format!(
                "To record to the cell, call its HTTP API (POST {url}/v1/decisions, /v1/evidence \
                 or /v1/hypotheses) or its MCP tools (POST {url}/mcp)"
            ),
            CellBesideVerb::Dump => format!(
                "To read the cell's graph, call its HTTP API (GET {url}/v1/graph; \
                 docs/KEYLESS_CAPTURE.md Step 5)"
            ),
            CellBesideVerb::Review => format!(
                "To read the cell's decisions, call GET {url}/v1/decisions/search; to act on one, \
                 POST {url}/v1/decisions/{{id}}/disagreements or /supersessions"
            ),
            CellBesideVerb::Query
            | CellBesideVerb::Digest
            | CellBesideVerb::Export
            | CellBesideVerb::Tui => format!(
                "To read the cell, call its HTTP API (GET {url}/v1/decisions/search, /recall or \
                 /relevant; docs/KEYLESS_CAPTURE.md Step 5)"
            ),
            CellBesideVerb::Map => format!(
                "To read the cell's decision map, call its HTTP API (GET {url}/v1/decisions/map)"
            ),
            CellBesideVerb::RestatementsPropose => format!(
                "To read the cell's proposals, call its HTTP API (GET \
                 {url}/v1/restatements/proposals)"
            ),
            CellBesideVerb::RestatementsApply => format!(
                "To link a restatement in the cell, call its HTTP API (POST \
                 {url}/v1/decisions/{{id}}/restatements, one link per call)"
            ),
            CellBesideVerb::Disagree => format!(
                "To disagree in the cell, call its HTTP API (POST \
                 {url}/v1/decisions/{{id}}/disagreements) or its MCP tool disagree_decision (POST \
                 {url}/mcp)"
            ),
            CellBesideVerb::Supersede => format!(
                "To supersede in the cell, call its HTTP API (POST \
                 {url}/v1/decisions/{{id}}/supersessions) or its MCP tool supersede_decision \
                 (POST {url}/mcp)"
            ),
            CellBesideVerb::Move => format!(
                "To move a decision in the cell, call its MCP tool move_decision (POST {url}/mcp)"
            ),
            CellBesideVerb::Retitle => format!(
                "To retitle a decision in the cell, call its MCP tool retitle_decision (POST \
                 {url}/mcp)"
            ),
            CellBesideVerb::Ground => format!(
                "To ground a decision in the cell, call its MCP tool ground_decision (POST \
                 {url}/mcp)"
            ),
            CellBesideVerb::Reaffirm => format!(
                "To say a decision still holds in the cell, call its HTTP API (POST \
                 {url}/v1/decisions/{{id}}/reaffirmations) or its MCP tool reaffirm_decision \
                 (POST {url}/mcp)"
            ),
            CellBesideVerb::Reexamine => format!(
                "To list the cell's re-examine items, call its MCP tool get_reexamine_items \
                 (POST {url}/mcp)"
            ),
            CellBesideVerb::Ask => {
                format!("To ask in the cell, call its MCP tool request_decision (POST {url}/mcp)")
            }
            CellBesideVerb::Import => format!(
                "The cell has no document import; to record a decision in it, call its HTTP API \
                 (POST {url}/v1/decisions) or its MCP tools (POST {url}/mcp)"
            ),
            CellBesideVerb::QualityScan => format!(
                "To scan the cell's decisions, call its MCP tool scan_decision_quality (POST \
                 {url}/mcp)"
            ),
            CellBesideVerb::ProjectRegister
            | CellBesideVerb::ProjectLink
            | CellBesideVerb::ProjectUnlink
            | CellBesideVerb::ProjectAnchor
            | CellBesideVerb::ProjectDeclareTopic => {
                "The cell has no HTTP route or MCP tool that changes its project registry"
                    .to_owned()
            }
            CellBesideVerb::ProjectList
            | CellBesideVerb::ProjectShow
            | CellBesideVerb::ProjectDecisions
            | CellBesideVerb::ProjectUse => format!(
                "The cell has no route for its project registry; to read its projects, call its \
                 HTTP API (GET {url}/v1/graph)"
            ),
            CellBesideVerb::IngestSlackThread => format!(
                "The cell takes Slack captures from Slack itself (POST {url}/v1/slack/events; \
                 docs/SLACK_APP.md); to record this thread's decision in it, call its HTTP API \
                 (POST {url}/v1/decisions) or its MCP tools (POST {url}/mcp)"
            ),
            CellBesideVerb::TenantCreate => format!(
                "To create a tenant in the cell, call its HTTP API (POST {url}/v1/tenants with \
                 the admin key; a Postgres cell only)"
            ),
            // Not reached beside a cell: these ask the server themselves (`honours_api_url`).
            CellBesideVerb::ClassifyQueueList | CellBesideVerb::ClassifyQueueSubmit => format!(
                "`hivemind classify-queue` asks the cell itself while HIVEMIND_API_URL is set \
                 (GET {url}/v1/classify-queue, POST {url}/v1/classify-queue/submit)"
            ),
        }
    }
}

/// A verb that reads or acts on a ledger, aimed at a folder that holds none. Opening the ledger
/// makes one, so a read under a mistyped `--hivemind-dir` (or a folder nothing was recorded in yet)
/// created an empty ledger on the spot and answered "No decision found", exit 0: indistinguishable
/// from a real ledger that has no match. A verb that acts on a decision or project (`disagree`,
/// `supersede`, `project link` and the rest) did the same and answered not_found, which sent the
/// user looking for a missing decision instead of a wrong folder. Neither can succeed on a ledger
/// that does not exist, so each refuses, names the folder and creates nothing. Writes keep making
/// the ledger on their first run; a configured `--database-url` names no folder; with
/// `HIVEMIND_API_URL` set the same refusal is [`LocalLedgerBesideCell`]'s, which also names the
/// cell. An existing ledger that holds nothing still answers (an acting verb says not_found).
/// `classify-queue` is the one verb that asks that server instead: with the variable set it opens
/// no ledger and is not refused, and with it unset it is refused like the rest, so the capture
/// plugin's `Stop` hook does not leave a ledger behind in every project it runs in.
fn refuse_verb_without_ledger(cli: &Cli) -> Result<()> {
    let Some(verb) = CellBesideVerb::of(&cli.command) else {
        return Ok(());
    };
    if verb.honours_api_url() && hivemind_api_url_from_env().is_some() {
        return Ok(());
    }
    let ledger_use = verb.ledger_use();
    if ledger_use == LedgerUse::WritesTo {
        return Ok(());
    }
    let config = LedgerConfig::from_cli(cli);
    if config
        .database_url
        .as_deref()
        .is_some_and(|url| !url.is_empty())
        || SqliteEventLedger::exists_in(&config.hivemind_dir)
    {
        return Ok(());
    }
    let dir = config.hivemind_dir.display();
    let verb = verb.name();
    let what_happened = match ledger_use {
        LedgerUse::Reads => format!(
            "`hivemind {verb}` reads a local ledger, and there is none under {dir}: nothing was \
             read and none was created, so this is not an answer of \"no decisions\"."
        ),
        LedgerUse::ReadsAndWrites => format!(
            "`hivemind {verb}` acts on what a local ledger already holds, and there is none under \
             {dir}: nothing was recorded and none was created, so there was nothing to act on and \
             this is not a \"not found\" for what you named."
        ),
        LedgerUse::WritesTo => return Ok(()),
    };
    Err(CliError::InvalidInput(format!(
        "{what_happened} Check --hivemind-dir (or HIVEMIND_DIR) for a typo and point it at the \
         folder that holds the ledger; a first `hivemind emit` there starts a new one"
    ))
    .into())
}

fn dispatch(cli: &Cli) -> Result<String> {
    match &cli.command {
        Command::Quickstart(args) => run_quickstart(cli, args),
        Command::Emit(command) => run_emit(cli, command),
        Command::Disagree(args) => run_disagree(cli, args),
        Command::Supersede(args) => run_supersede(cli, args),
        Command::Move(args) => run_move(cli, args),
        Command::Retitle(args) => run_retitle(cli, args),
        Command::Ground(args) => ground::run_ground(cli, args),
        Command::Reaffirm(args) => reaffirm::run_reaffirm(cli, args),
        Command::Reexamine(args) => reaffirm::run_reexamine(cli, args),
        Command::Ask(args) => ask::run_ask(cli, args),
        Command::Review(args) => run_review(cli, args),
        Command::Import(import) => run_import(cli, import),
        Command::Query(query) => run_query(cli, query),
        Command::Dump(dump) => run_dump(cli, dump),
        Command::Tui(args) => run_tui(cli, args),
        Command::Ingest(args) => run_ingest(cli, args),
        Command::SlackApp(args) => run_slack_app(cli, args),
        Command::Mcp(args) => run_mcp(cli, args),
        Command::Serve(args) => run_serve(cli, args),
        Command::Migrate(args) => migrate::run_migrate(cli, args),
        Command::Map(args) => run_map(cli, args),
        Command::Digest(args) => run_digest(cli, args),
        Command::ClassifyQueue(args) => run_classify_queue(cli, args),
        Command::Restatements(args) => run_restatements(cli, args),
        Command::Connector(args) => run_connector(cli, args),
        Command::QualityScan(args) => run_quality_scan(cli, args),
        Command::Export(args) => run_export(cli, args),
        Command::Tenant(args) => run_tenant(cli, args),
        Command::Project(args) => run_project(cli, args),
    }
}

fn run_quickstart(cli: &Cli, _args: &QuickstartArgs) -> Result<String> {
    // Isolated ledger directory (never the caller's --hivemind-dir) so quickstart
    // never touches a real local ledger. When --database-url/HIVEMIND_DATABASE_URL
    // is configured, backend selection still applies — quickstart then proves the
    // configured Postgres backend works end to end instead of silently writing a
    // throwaway local SQLite file that has nothing to do with it; the isolated
    // directory is simply unused on that path.
    let ledger_dir = std::env::temp_dir().join(format!("hivemind-quickstart-{}", Uuid::new_v4()));
    let tenant_id = cli_tenant(cli)?;
    let mut ledger_config = LedgerConfig::from_cli(cli);
    ledger_config.hivemind_dir = ledger_dir.clone();
    let ledger = AnyLedger::open(&ledger_config, &tenant_id)?;
    let commands = Commands::new_with_context(
        &ledger,
        CommandContext::new(tenant_id.clone(), EventProvenance::cli()),
    );
    let decision_args = EmitDecisionProposedArgs {
        title: "Try HiveMind quickstart".to_owned(),
        rationale: "A first decision should be captured with actor provenance and queried back immediately.".to_owned(),
        topic_keys: vec!["quickstart".to_owned(), "onboarding".to_owned()],
        options: OptionLabelArgs {
            listed: vec!["Local ledger".to_owned(), "Spreadsheet".to_owned()],
            single: Vec::new(),
        },
        chosen_option_id: Some("Local ledger".to_owned()),
        decided_by: None,
        delegated_by: None,
        still_proposed: false,
        hypothesis_ids: Vec::new(),
        evidence_ids: Vec::new(),
        quote: None,
        question: None,
        project: None,
        project_source: None,
        project_from_context: false,
        declare_topics: Vec::new(),
    };
    let (decision_id, _placement) =
        propose_decision_from_option_labels(&commands, &cli.actor, &decision_args, None)?;

    let graph = MemoryGraph::default();
    rebuild_graph_for_tenant(&ledger, &tenant_id, &graph)?;
    let query = search_decisions(
        &graph,
        &SearchDecisionRequest {
            query: Some("quickstart".to_owned()),
            topic_keys: vec!["quickstart".to_owned()],
            // hivemind-zdsh.8: a chosen option self-accepts by default, so the demo
            // decision captured above lands as `accepted`, not `proposed`.
            statuses: vec![DecisionStatus::Accepted],
            actor_ids: vec![cli.actor.clone()],
            sources: vec!["cli".to_owned()],
            since: None,
            until: None,
            limit: 5,
            cursor: None,
            project: None,
        },
    )?;
    let first_result_id = query
        .data
        .items
        .first()
        .map(|item| item.decision.id.clone());

    // ubs:ignore: decision id equality check, not a secret/token comparison
    if first_result_id.as_deref() != Some(decision_id.as_str()) {
        return Err(CliError::InvalidInput(
            "quickstart query did not return captured decision".to_owned(),
        )
        .into());
    }

    let report = QuickstartReport {
        ledger_dir: ledger_dir.display().to_string(),
        actor_id: cli.actor.clone(),
        decision_id,
        query: QuickstartQueryReport {
            result_count: query.result_count,
            total_matches: query.data.total_matches,
            truncated: query.truncated,
            first_result_id,
        },
    };

    if cli.json {
        format_json_value(true, &report)
    } else {
        Ok(format_quickstart_report(&report))
    }
}

fn format_quickstart_report(report: &QuickstartReport) -> String {
    format!(
        "HiveMind quickstart complete.\n\
         Ledger: {ledger_dir}\n\
         Actor: {actor_id}\n\
         Captured: {decision_id}\n\
         Queried: found {first_result_id} ({result_count} result, truncated={truncated})\n\n\
         Try the query again:\n\
           hivemind --hivemind-dir {ledger_dir} query search_decisions --topic quickstart --limit 5",
        ledger_dir = report.ledger_dir,
        actor_id = report.actor_id,
        decision_id = report.decision_id,
        first_result_id = report
            .query
            .first_result_id
            .as_deref()
            .unwrap_or("<missing>"),
        result_count = report.query.result_count,
        truncated = report.query.truncated
    )
}

fn run_mcp(cli: &Cli, args: &McpArgs) -> Result<String> {
    let mut config = crate::mcp::McpConfig::new(cli.hivemind_dir.clone())
        .with_tenant(cli_tenant(cli)?)
        .with_database_url(cli.database_url.clone());
    if let Some(agent_tool) = args.agent_tool.as_deref().map(str::trim) {
        if !agent_tool.is_empty() {
            config = config.with_agent_tool(agent_tool);
        }
    }
    if let Some(session_id) = args.session_id.as_deref().map(str::trim) {
        if !session_id.is_empty() {
            config = config.with_session_id(session_id);
        }
    }
    if args.project_from_context {
        config = config.with_project_context(ProjectContextEnv::from_process(&cli.person()));
    }
    crate::mcp::serve_stdio(&config)?;
    // The stdio loop only returns once stdin closes — no payload to print.
    Ok(String::new())
}

fn run_serve(cli: &Cli, args: &ServeArgs) -> Result<String> {
    let config = crate::api::ApiConfig::new(cli.hivemind_dir.clone())
        .with_bind(args.bind)
        .with_port(args.port)
        .with_allow_unauthenticated_remote(args.allow_unauthenticated_remote)
        .with_database_url(LedgerConfig::from_cli(cli).database_url);
    // Build AppState (which constructs r2d2/postgres pool) BEFORE entering
    // the tokio runtime. r2d2 pool construction internally calls block_on,
    // which panics if already inside an existing runtime.
    let state = crate::api::AppState::from_config(&config)?;
    // Hold a clone so the Arc<ApiBackend> (postgres pool) survives until AFTER
    // the runtime is dropped. Rust drops locals in reverse declaration order:
    // `runtime` (declared below) drops before `_pg_guard`, so the pool's Drop
    // runs outside any runtime context — preventing the block_on-within-block_on
    // SIGABRT on scale-to-zero autostop (hivemind-noc9).
    let _pg_guard = state.clone();
    let runtime = tokio::runtime::Runtime::new()
        .map_err(|e| CliError::InvalidInput(format!("failed to create tokio runtime: {e}")))?;
    runtime.block_on(crate::api::serve_http(state, &config))?;
    Ok(String::new())
}

fn run_map(cli: &Cli, args: &MapArgs) -> Result<String> {
    let tenant_id = cli_tenant(cli)?;
    let ledger = open_ledger(cli)?;
    let graph = MemoryGraph::default();
    rebuild_graph_for_tenant(&ledger, &tenant_id, &graph)?;

    let alphas: Vec<f64> = if args.alpha.is_empty() {
        vec![0.5]
    } else {
        args.alpha.clone()
    };

    if alphas.len() == 1 {
        let result = crate::map::compute_map(&graph, Some(cli.hivemind_dir.as_path()), alphas[0]) // ubs:ignore: alphas[0] guarded by len()==1 check above
            .map_err(|e| CliError::InvalidInput(e.to_string()))?; // ubs:ignore: error conversion at CLI boundary
        if args.summary {
            let mut out = format!(
                "Decision map: {} decisions, alpha={:.2}, gen={}\n", // ubs:ignore: format! for CLI output
                result.n,
                result.alpha,
                &result.gen_id[..8] // ubs:ignore: UUID is 36 chars; 8-char prefix always in-bounds
            );
            let points: String = result
                .points
                .iter()
                .map(|p| {
                    format!(
                        "  [{:>6.2}, {:>6.2}] {:8} {}\n",
                        p.x_time, p.y_spectral, p.status, p.title
                    )
                })
                .collect();
            out.push_str(&points);
            Ok(out)
        } else {
            serde_json::to_string_pretty(&result)
                .map_err(|e| CliError::InvalidInput(e.to_string()).into()) // ubs:ignore: error conversion at CLI boundary
        }
    } else {
        let mut results = Vec::new();
        for &alpha in &alphas {
            let r = crate::map::compute_map(&graph, Some(cli.hivemind_dir.as_path()), alpha)
                .map_err(|e| CliError::InvalidInput(e.to_string()))?; // ubs:ignore: error conversion at CLI boundary
            results.push(r);
        }
        serde_json::to_string_pretty(&results)
            .map_err(|e| CliError::InvalidInput(e.to_string()).into()) // ubs:ignore: error conversion at CLI boundary
    }
}

fn parse_window_duration(window: &str) -> Result<chrono::Duration> {
    let window = window.trim();
    let (digits, unit) = window.split_at(
        window
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(window.len()), // ubs:ignore: unwrap_or — safe default: all-digits treated as days
    );
    let n: i64 = digits
        .parse()
        .map_err(|_| CliError::InvalidInput(format!("--window: invalid duration '{window}'")))?;
    if n <= 0 {
        return Err(CliError::InvalidInput(format!(
            "--window: duration must be positive, got '{window}'"
        ))
        .into());
    }
    match unit {
        "h" => Ok(chrono::Duration::hours(n)),
        "d" | "" => Ok(chrono::Duration::days(n)),
        "w" => Ok(chrono::Duration::weeks(n)),
        other => Err(CliError::InvalidInput(format!(
            "--window: unknown unit '{other}'; use h (hours), d (days), or w (weeks)"
        ))
        .into()),
    }
}

fn run_digest(cli: &Cli, args: &DigestArgs) -> Result<String> {
    let tenant_id = cli_tenant(cli)?;
    let ledger = open_ledger(cli)?;
    let context = QueryContext::new(tenant_id.clone());
    let graph = MemoryGraph::default();
    rebuild_graph_for_tenant(&ledger, &tenant_id, &graph)?;

    let now = Utc::now();

    let until = match args.until.as_deref() {
        Some(s) => parse_required_query_datetime(s, "--until")?,
        None => now,
    };
    let since = match args.since.as_deref() {
        Some(s) => parse_required_query_datetime(s, "--since")?,
        None => {
            let duration = parse_window_duration(&args.window)?;
            until - duration
        }
    };

    let request = DigestRequest {
        since,
        until,
        actor_ids: args.actor_ids.clone(), // ubs:ignore: clone necessary — building owned DigestRequest from borrowed DigestArgs
        limit: args.limit,
        cursor: args.cursor.clone(), // ubs:ignore: clone necessary — building owned DigestRequest from borrowed DigestArgs
    };
    let response = weekly_digest(&context, &ledger, &graph, &request)?;

    if args.summary {
        let mut out = response.data.text.clone(); // ubs:ignore: clone necessary — formatting owned output from borrowed DigestResponse
        append_truncation_notice(
            &mut out,
            response.truncated,
            response.data.next_cursor.as_deref(),
        );
        Ok(out.trim_end().to_owned())
    } else {
        format_json_value(true, &response)
    }
}

fn run_restatements(cli: &Cli, args: &RestatementsArgs) -> Result<String> {
    let tenant_id = cli_tenant(cli)?;
    let ledger = open_ledger(cli)?;
    let graph = MemoryGraph::default();
    rebuild_graph_for_tenant(&ledger, &tenant_id, &graph)?;

    match &args.command {
        RestatementsCommand::Propose(args) => {
            let proposals =
                crate::restatement::propose_same_as_links(&graph, args.project.as_deref())?;
            if cli.json {
                format_json_value(true, &proposals)
            } else {
                Ok(render_restatement_proposals(&proposals))
            }
        }
        RestatementsCommand::Apply(args) => {
            let links = if args.all {
                crate::restatement::propose_same_as_links(&graph, args.project.as_deref())?
                    .proposed
                    .into_iter()
                    .map(|link| (link.decision_id, link.restates_id))
                    .collect()
            } else if args.link.is_empty() {
                return Err(CliError::InvalidInput(
                    "name the links to record with --link LATER=EARLIER, or record every proposed link with --all".to_owned(),
                )
                .into());
            } else {
                args.link
                    .iter()
                    .map(|link| parse_restatement_link(link))
                    .collect::<Result<Vec<_>>>()?
            };
            let commands = Commands::new_with_context(
                &ledger,
                CommandContext::new(tenant_id, fluent_write_provenance(&cli.actor)),
            );
            let applied = crate::restatement::apply_links(&commands, &cli.actor, &links)?;
            if cli.json {
                format_json_value(true, &applied)
            } else {
                Ok(render_applied_links(&applied))
            }
        }
    }
}

/// `LATER=EARLIER` as `(later, earlier)`.
fn parse_restatement_link(link: &str) -> Result<(String, String)> {
    match link.split_once('=') {
        Some((later, earlier)) if !later.trim().is_empty() && !earlier.trim().is_empty() => {
            Ok((later.trim().to_owned(), earlier.trim().to_owned()))
        }
        _ => Err(CliError::InvalidInput(format!(
            "--link {link:?} is not LATER=EARLIER: two decision ids joined by ="
        ))
        .into()),
    }
}

fn run_classify_queue(cli: &Cli, args: &ClassifyQueueArgs) -> Result<String> {
    match &args.command {
        ClassifyQueueCommand::List(args) => run_classify_queue_list(cli, args),
        ClassifyQueueCommand::Submit(args) => run_classify_queue_submit(cli, args),
    }
}

fn run_classify_queue_list(cli: &Cli, args: &ClassifyQueueListArgs) -> Result<String> {
    if let Some(server) = ClassifyQueueServer::from_env(cli)? {
        return server.list(cli.json, args);
    }

    let tenant_id = cli_tenant(cli)?;
    let ledger = open_ledger(cli)?;
    let queue = crate::classifier::pending_queue_for_ledger(
        &ledger,
        &tenant_id,
        args.session_id.as_deref(),
        args.limit,
    )
    .map_err(|e| CliError::InvalidInput(format!("ledger scan failed: {e}")))?;
    format_json_value(cli.json, &queue.to_json())
}

fn run_classify_queue_submit(cli: &Cli, args: &ClassifyQueueSubmitArgs) -> Result<String> {
    let captures: Vec<CaptureItem> = serde_json::from_str(&args.captures)
        .map_err(|e| CliError::InvalidInput(format!("--captures is not valid JSON: {e}")))?;

    if let Some(server) = ClassifyQueueServer::from_env(cli)? {
        return server.submit(cli.json, args, captures);
    }

    let tenant_id = cli_tenant(cli)?;
    let ledger = open_ledger(cli)?;

    let cap_status = crate::classifier::daily_cap_status(&ledger, &tenant_id)
        .map_err(|e| CliError::InvalidInput(format!("ledger scan failed: {e}")))?;
    let commands = Commands::new_with_context(
        &ledger,
        CommandContext::new(tenant_id, EventProvenance::cli()),
    );

    // A batch this ledger never received is refused whatever the cap says: nothing is waiting
    // under that id, so "wait until tomorrow" would be the wrong advice.
    commands.require_received_batches(&args.batch_id)?;
    if cap_status.remaining == 0 {
        return Err(CliError::InvalidInput(format!(
            "daily classification cap ({}) reached; batches remain pending until tomorrow (UTC)",
            cap_status.cap
        ))
        .into());
    }

    let recorded = commands.record_ingest_batch_classified(
        &cli.actor,
        &args.batch_id,
        &args.model,
        crate::classifier::SCHEMA_VERSION,
        captures,
        None,
    )?;

    let mut result = serde_json::json!({
        "batch_ids": args.batch_id,
        "capture_count": recorded.recorded_count,
    });
    recorded.annotate_reply(&mut result);
    format_json_value(cli.json, &result)
}

/// `classify-queue list`/`submit` talk to a server-backed cell over HTTP
/// instead of opening a local ledger when `HIVEMIND_API_URL` is set (Option
/// B, hivemind-zdsh.2: agents on a shared cell never hold a direct DB
/// credential — see docs/AGENT_DECISION_CAPTURE.md's `HIVEMIND_API_URL`/
/// `HIVEMIND_API_KEY` convention, shared with the Python capture clients).
/// `HIVEMIND_API_URL` unset selects the local SQLite path, unchanged.
///
/// Every request carries `--tenant` and `--actor` as `X-HiveMind-Tenant` and `X-HiveMind-Actor`,
/// so a server that takes both from the caller (SQLite, with a static key or none) reads and
/// writes the tenant and actor the flags name, as a local ledger does. A server that knows the
/// caller by token takes tenant and actor from the token and ignores the headers.
struct ClassifyQueueServer {
    base_url: String,
    token: Option<String>,
    tenant: TenantId,
    actor: String,
}

/// The server a cell user pointed the CLI at: `HIVEMIND_API_URL`, trimmed, without a trailing
/// slash; `None` when it is unset or blank.
fn hivemind_api_url_from_env() -> Option<String> {
    let base_url = std::env::var("HIVEMIND_API_URL").ok()?;
    let base_url = base_url.trim();
    if base_url.is_empty() {
        return None;
    }
    Some(base_url.trim_end_matches('/').to_owned())
}

impl ClassifyQueueServer {
    fn from_env(cli: &Cli) -> Result<Option<Self>> {
        let Some(base_url) = hivemind_api_url_from_env() else {
            return Ok(None);
        };
        let token = std::env::var("HIVEMIND_API_KEY")
            .ok()
            .filter(|t| !t.trim().is_empty());
        Ok(Some(Self {
            base_url,
            token,
            tenant: cli_tenant(cli)?,
            actor: cli.actor.clone(),
        }))
    }

    fn client(&self) -> Result<reqwest::blocking::Client> {
        reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| CliError::InvalidInput(format!("failed to build HTTP client: {e}")).into())
    }

    fn authed(
        &self,
        builder: reqwest::blocking::RequestBuilder,
    ) -> reqwest::blocking::RequestBuilder {
        let builder = builder
            .header(crate::api::HEADER_TENANT, self.tenant.as_str())
            .header(crate::api::HEADER_ACTOR, &self.actor);
        match &self.token {
            Some(token) => builder.bearer_auth(token),
            None => builder,
        }
    }

    fn list(&self, json_output: bool, args: &ClassifyQueueListArgs) -> Result<String> {
        let client = self.client()?;
        let url = format!("{}/v1/classify-queue", self.base_url);
        let mut query: Vec<(&str, String)> = vec![("limit", args.limit.to_string())];
        if let Some(ref session_id) = args.session_id {
            query.push(("session_id", session_id.clone()));
        }
        let resp = self
            .authed(client.get(&url).query(&query))
            .send()
            .map_err(|e| {
                CliError::InvalidInput(format!("classify-queue list request failed: {e}"))
            })?;
        http_response_to_cli_output(resp, json_output)
    }

    fn submit(
        &self,
        json_output: bool,
        args: &ClassifyQueueSubmitArgs,
        captures: Vec<CaptureItem>,
    ) -> Result<String> {
        let client = self.client()?;
        let url = format!("{}/v1/classify-queue/submit", self.base_url);
        let body = serde_json::json!({
            "batch_ids": args.batch_id,
            "captures": captures,
            "model": args.model,
        });
        let resp = self
            .authed(client.post(&url).json(&body))
            .send()
            .map_err(|e| {
                CliError::InvalidInput(format!("classify-queue submit request failed: {e}"))
            })?;
        http_response_to_cli_output(resp, json_output)
    }
}

fn http_response_to_cli_output(
    resp: reqwest::blocking::Response,
    json_output: bool,
) -> Result<String> {
    let status = resp.status();
    let value: serde_json::Value = resp
        .json()
        .map_err(|e| CliError::InvalidInput(format!("invalid JSON response: {e}")))?;
    if !status.is_success() {
        let message = value
            .get("error")
            .and_then(|e| e.get("message"))
            .and_then(|m| m.as_str())
            .unwrap_or("request failed")
            .to_owned();
        // The server answers a refusal that names something it does not hold (a batch id it
        // never received) with 404 `not_found`; it maps an invariant violation to that status
        // (`to_api_error`). The same refusal on a local ledger is that invariant violation, exit
        // 3, so it comes back as one here, in the server's words: an agent that branches on the
        // exit code sees one answer whether it asked a ledger or a cell. The input was fine, so it
        // is not "invalid command line input". A 404 without the server's `not_found` body (a
        // proxy, the wrong host) stays the generic refusal below.
        //
        // A token that maps to a tenant the server does not hold is a different refusal, 404
        // `unknown_tenant`: every call fails the same way for one configuration reason. On a local
        // ledger that is a storage error ("unknown tenant '<id>': <hint>"), exit 4, so it comes
        // back as one here. Exit 3 would read as "that one batch is unknown" and send an agent on
        // to the next id.
        let code = value
            .get("error")
            .and_then(|e| e.get("code"))
            .and_then(|c| c.as_str());
        if status == reqwest::StatusCode::NOT_FOUND {
            match code {
                Some("not_found") => return Err(CommandError::Invariant(message).into()),
                Some("unknown_tenant") => return Err(LedgerError::Storage(message).into()),
                _ => {}
            }
        }
        return Err(CliError::InvalidInput(format!("server returned {status}: {message}")).into());
    }
    format_json_value(json_output, &value)
}

fn run_connector(cli: &Cli, args: &ConnectorArgs) -> Result<String> {
    match &args.command {
        ConnectorCommand::Auth(auth_args) => run_connector_auth(cli, auth_args),
    }
}

fn run_connector_auth(cli: &Cli, args: &ConnectorAuthArgs) -> Result<String> {
    match args.connector.as_str() {
        "gdocs" | "google" | "google-docs" | "googledocs" => run_connector_auth_gdocs(cli),
        other => Err(CliError::InvalidInput(format!(
            "unknown connector '{other}'; supported: gdocs"
        ))
        .into()),
    }
}

fn run_connector_auth_gdocs(cli: &Cli) -> Result<String> {
    let client_id = std::env::var("HIVEMIND_GOOGLE_CLIENT_ID").map_err(|_| {
        CliError::InvalidInput(
            "HIVEMIND_GOOGLE_CLIENT_ID environment variable must be set".to_owned(),
        )
    })?;
    let client_secret = std::env::var("HIVEMIND_GOOGLE_CLIENT_SECRET").map_err(|_| {
        CliError::InvalidInput(
            "HIVEMIND_GOOGLE_CLIENT_SECRET environment variable must be set".to_owned(),
        )
    })?;

    let runtime = tokio::runtime::Runtime::new()
        .map_err(|e| CliError::InvalidInput(format!("failed to create async runtime: {e}")))?;

    let (auth_code, redirect_uri) =
        runtime.block_on(crate::connector::listen_for_google_oauth_code(&client_id))?;

    let token_store = crate::connector::GoogleTokenStore::new(&cli.hivemind_dir);
    crate::connector::exchange_google_oauth_code(
        &client_id,
        &client_secret,
        &auth_code,
        &redirect_uri,
        &token_store,
    )?;

    Ok(
        "Google Docs connector authenticated successfully. Token saved to connector-tokens.json."
            .to_owned(),
    )
}

fn run_ingest(cli: &Cli, ingest: &IngestArgs) -> Result<String> {
    match &ingest.command {
        IngestCommand::SlackThread(args) => run_ingest_slack_thread(cli, args),
    }
}

fn run_ingest_slack_thread(cli: &Cli, args: &IngestSlackThreadArgs) -> Result<String> {
    let contents = std::fs::read_to_string(&args.file).map_err(|error| {
        CliError::InvalidInput(format!(
            "could not read slack thread file {}: {error}",
            args.file.display()
        ))
    })?;
    let thread = parse_slack_thread_fixture(&contents)?;
    let draft = extract_slack_decision_draft(&thread, &args.mention)?;

    let ledger = open_ledger(cli)?;
    let scoped_ledger = TenantScopedLedger::new(&ledger, cli_tenant(cli)?);
    let outcome = import_slack_thread(&scoped_ledger, &draft)?;

    let kind = match outcome {
        SlackIngestOutcome::Imported { .. } => "decision_id",
        SlackIngestOutcome::AlreadyImported { .. } => "decision_id_existing",
    };
    let envelope = OutputEnvelope::new("ingest", kind, outcome.decision_id().to_owned());
    format_output(cli.json, &envelope)
}

/// The tenant a Slack workspace maps onto: its team id, 1:1, as the server's Slack routes
/// use it. It must already be registered (`hivemind tenant create <team_id>`).
fn slack_team_tenant(team_id: &str) -> Result<TenantId> {
    TenantId::new(team_id).map_err(|error| {
        CliError::InvalidInput(format!("Slack team id is invalid: {error}")).into()
    })
}

fn run_slack_app(cli: &Cli, args: &SlackAppArgs) -> Result<String> {
    let store = SlackAppStore::new(&cli.hivemind_dir);
    match &args.command {
        SlackAppCommand::Manifest(args) => {
            let manifest = slack_app_manifest(
                &args.request_url,
                args.event_url.as_deref(),
                args.interactivity_url.as_deref(),
                args.redirect_url.as_deref(),
            )?;
            format_json_value(cli.json, &manifest)
        }
        SlackAppCommand::OauthUrl(args) => {
            slack_oauth_install_url(&args.client_id, &args.redirect_uri, &args.state)
        }
        SlackAppCommand::Install(args) => {
            let summary = store.install_workspace(SlackWorkspaceInstall {
                team_id: args.team_id.clone(),
                team_name: args.team_name.clone(),
                bot_token: args.bot_token.clone(),
                signing_secret: args.signing_secret.clone(),
                hivemind_url: args.hivemind_url.clone(),
                reaction_emoji: args.reaction_emoji.clone(),
                actor_mappings: parse_actor_mappings(&args.actor_mappings)?,
            })?;
            format_json_value(cli.json, &summary)
        }
        SlackAppCommand::EnqueueCapture(args) => {
            let event = store.enqueue_capture(SlackCaptureRequest {
                team_id: args.team_id.clone(),
                user_id: args.user_id.clone(),
                decided_by_user_id: None,
                channel_id: args.channel_id.clone(),
                message_ts: args.message_ts.clone(),
                thread_ts: args
                    .thread_ts
                    .clone()
                    .unwrap_or_else(|| args.message_ts.clone()),
                permalink: args.permalink.clone(),
                surface: args.surface.as_slack_surface(),
                reaction_emoji: args.reaction_emoji.clone(),
                title: args.title.clone(),
                rationale: args.rationale.clone(),
                topic_keys: args.topic_keys.clone(),
                option_labels: args.options.labels(),
                chosen_option_label: args.chosen_option_label.clone(),
                thread_text: args.thread_text.clone(),
            })?;
            format_json_value(cli.json, &event)
        }
        SlackAppCommand::Drain(_) => {
            // Each capture is written into its own workspace's tenant, as the server's drain
            // does; the global `--tenant` does not choose where a team's captures go.
            let config = LedgerConfig::from_cli(cli);
            let report = store.drain_queue_multi_tenant(|team_id| {
                let tenant_id = slack_team_tenant(team_id)?;
                let ledger = AnyLedger::open(&config, &tenant_id)?;
                Ok(TenantScopedOwnedLedger::new(ledger, tenant_id))
            })?;
            format_json_value(cli.json, &report)
        }
        SlackAppCommand::Command(args) => {
            let tenant_id = slack_team_tenant(&args.team_id)?;
            let ledger = AnyLedger::open(&LedgerConfig::from_cli(cli), &tenant_id)?;
            let scoped_ledger = TenantScopedLedger::new(&ledger, tenant_id.clone());
            let graph = MemoryGraph::default();
            rebuild_graph_for_tenant(&ledger, &tenant_id, &graph)?;
            let response = handle_slack_command(
                &scoped_ledger,
                &graph,
                &store,
                &SlackCommandRequest {
                    team_id: args.team_id.clone(),
                    user_id: args.user_id.clone(),
                    text: args.text.clone(),
                    limit: args.limit,
                },
            )?;
            format_json_value(cli.json, &response)
        }
    }
}

fn run_emit(cli: &Cli, emit: &EmitArgs) -> Result<String> {
    run_emit_with_notices(cli, emit, &mut io::stderr())
}

/// `run_emit`, with the text-mode project announcement (see `announce_placement`) written to
/// `notices` rather than straight to stderr, so a test can read it.
pub(crate) fn run_emit_with_notices<W: IoWrite>(
    cli: &Cli,
    emit: &EmitArgs,
    notices: &mut W,
) -> Result<String> {
    run_emit_in_context(
        cli,
        emit,
        notices,
        &ProjectContextEnv::from_process(&cli.person()),
    )
}

/// `run_emit_with_notices` with the surroundings `--project-from-context` reads -- working
/// directory, rig, acting person -- supplied instead of taken from the process, so a test can
/// run a capture from any folder without touching process-global state.
pub(crate) fn run_emit_in_context<W: IoWrite>(
    cli: &Cli,
    emit: &EmitArgs,
    notices: &mut W,
    env: &ProjectContextEnv,
) -> Result<String> {
    let ledger = open_ledger(cli)?;
    let commands = Commands::new_with_context(
        &ledger,
        cli_command_context(cli, cli_emit_provenance(&cli.actor))?,
    );

    let output = match &emit.command {
        EmitCommand::DecisionCapture(args) => {
            let (actor_id, provenance) = decision_capture_actor_and_provenance(
                cli,
                &args.provenance,
                agent_present_in_env(),
            )?;
            let commands =
                Commands::new_with_context(&ledger, cli_command_context(cli, provenance)?)
                    .declaring_topics(&args.decision.declare_topics);
            let spec = grounding::require_grounding(grounding::grounding_spec_from_args(
                &cli.hivemind_dir,
                &args.grounding,
                &args.decision.evidence_ids,
                &args.decision.hypothesis_ids,
            )?)?;
            // Refused before anything resolves or is written, like the grounding above.
            let floor_answers = floor::floor_answers_from_args(&args.floor)?;
            let resolved = grounding::resolve_declared_grounding(
                &cli.hivemind_dir,
                &ledger,
                &cli_tenant(cli)?,
                spec,
            )?;
            let option_labels = args.decision.options.labels();
            let (option_ids, chosen_option_id) = record_cli_options(
                &commands,
                &actor_id,
                &option_labels,
                args.decision.chosen_option_id.as_deref(),
            )?;
            let project = cli_capture_project(cli, &ledger, env, &args.decision)?;
            let question = commands.resolve_answers_request(
                args.answers_request_id.as_deref(),
                args.decision.question.as_deref(),
            )?;
            let proposal = commands.propose_grounded_decision(
                DecisionProposalInput {
                    actor_id: &actor_id,
                    title: &args.decision.title,
                    rationale: &args.decision.rationale,
                    topic_keys: &args.decision.topic_keys,
                    option_ids: &option_ids,
                    option_labels: &option_labels,
                    chosen_option_id: chosen_option_id.as_deref(),
                    decided_by: args.decision.decided_by.as_deref(),
                    delegated_by: args.decision.delegated_by.as_deref(),
                    still_proposed: args.decision.still_proposed,
                    // The plan carries every id; see `propose_grounded_decision`.
                    hypothesis_ids: &[],
                    evidence_ids: &[],
                    quote: args.decision.quote.as_deref(),
                    question: question.as_deref(),
                    grounding: Grounding::NotAsked,
                    expressed_confidence: args.grounding.confidence.as_deref(),
                    project: determined_project(&project),
                    floor: FloorInput::Stated(&floor_answers),
                },
                &resolved.plan,
            )?;
            let floor_reply = floor_answers.reply();
            let reminder = project_reminder(&project, &proposal.placement);
            announce_placement(cli, &proposal.placement, reminder.as_deref(), notices);
            floor::announce_below_floor(cli, &floor_reply, notices);
            announce_cited_not_linked(
                cli,
                &proposal.decision_id,
                &proposal.cited_not_linked,
                notices,
            );
            announce_warnings(cli, &proposal.warnings, notices);
            return format_capture_output(
                cli.json,
                &CaptureCommandOutput {
                    subcommand: "emit",
                    kind: "decision_id",
                    value: proposal.decision_id,
                    project_notice: proposal.placement.notice(),
                    project_reminder: reminder,
                    placement: proposal.placement,
                    rests_on: resolved.label(proposal.rests_on),
                    premise_stale: proposal.premise_stale,
                    cited_not_linked: proposal.cited_not_linked,
                    warnings: proposal.warnings,
                    question_id: proposal.question.map(|question| question.question_id),
                    below_floor: floor_reply.below_floor(),
                    below_floor_note: floor_reply.note(),
                    floor: floor_reply,
                },
            );
        }
        EmitCommand::DecisionProposed(args) => {
            let commands = Commands::new_with_context(
                &ledger,
                cli_command_context(cli, cli_emit_provenance(&cli.actor))?,
            )
            .declaring_topics(&args.declare_topics);
            let project = cli_capture_project(cli, &ledger, env, args)?;
            let (decision_id, placement) = propose_decision_from_option_labels(
                &commands,
                &cli.actor,
                args,
                determined_project(&project),
            )?;
            let reminder = project_reminder(&project, &placement);
            OutputEnvelope::new("emit", "decision_id", decision_id)
                .with_placement(placement)
                .with_project_reminder(reminder)
        }
        EmitCommand::DecisionAccepted(args) => {
            let event_id = commands.accept_decision(&args.decision_id, &cli.actor)?;
            OutputEnvelope::new("emit", "event_id", event_id.to_string())
        }
        EmitCommand::DecisionRejected(args) => {
            let event_id = commands.reject_decision(&args.decision_id, &cli.actor)?;
            OutputEnvelope::new("emit", "event_id", event_id.to_string())
        }
        EmitCommand::DecisionSuperseded(args) => {
            let event_id = commands.supersede_decision(
                &args.old_decision_id,
                &args.new_decision_id,
                &cli.actor,
            )?;
            OutputEnvelope::new("emit", "event_id", event_id.to_string())
        }
        EmitCommand::EvidenceRecorded(args) => {
            let (actor_id, commands) = emit_actor_and_commands(cli, &ledger, &args.provenance)?;
            let evidence_id = commands.record_evidence(&actor_id, &args.content)?;
            OutputEnvelope::new("emit", "evidence_id", evidence_id)
        }
        EmitCommand::HypothesisRecorded(args) => {
            let (actor_id, commands) = emit_actor_and_commands(cli, &ledger, &args.provenance)?;
            let kind = match args.kind {
                EmitHypothesisKind::Assumption => HypothesisKind::Assumption,
                EmitHypothesisKind::Bet => HypothesisKind::Bet,
            };
            let check_by = args
                .check_by
                .as_deref()
                .map(|value| parse_check_by_date("--check-by", value))
                .transpose()?;
            let hypothesis_id = commands.record_hypothesis_with_kind(
                &actor_id,
                &args.statement,
                kind,
                check_by,
                args.would_change_if.as_deref(),
            )?;
            OutputEnvelope::new("emit", "hypothesis_id", hypothesis_id)
        }
        EmitCommand::OptionRecorded(args) => {
            let option_id = commands.record_option(&cli.actor, &args.label, &args.description)?;
            OutputEnvelope::new("emit", "option_id", option_id)
        }
        EmitCommand::RelationAdded(args) => {
            let event_id = match args.kind {
                EmitRelationKind::BasedOn => {
                    commands.attach_evidence(&args.from_id, &args.to_id, &cli.actor)?
                }
                EmitRelationKind::Supports => commands.relate_evidence_to_hypothesis(
                    &args.from_id,
                    &args.to_id,
                    EventRelationKind::Supports,
                    &cli.actor,
                )?,
                EmitRelationKind::Refutes => commands.relate_evidence_to_hypothesis(
                    &args.from_id,
                    &args.to_id,
                    EventRelationKind::Refutes,
                    &cli.actor,
                )?,
                EmitRelationKind::FollowsFrom => {
                    commands.link_follows_from(&args.from_id, &args.to_id, &cli.actor)?
                }
            };

            OutputEnvelope::new("emit", "event_id", event_id.to_string())
        }
        EmitCommand::AttachEvidence(args) => {
            let event_id =
                commands.attach_evidence(&args.decision_id, &args.evidence_id, &cli.actor)?;
            OutputEnvelope::new("emit", "event_id", event_id.to_string())
        }
        EmitCommand::IngestBatchClassified(args) => {
            let (actor_id, provenance) = capture_actor_and_provenance(&args.provenance)?;
            let commands =
                Commands::new_with_context(&ledger, cli_command_context(cli, provenance)?);
            let json_text = std::fs::read_to_string(&args.captures_file).map_err(|e| {
                CliError::InvalidInput(format!(
                    "cannot read captures file {:?}: {e}",
                    args.captures_file
                ))
            })?;
            let captures: Vec<CaptureItem> = serde_json::from_str(&json_text)
                .map_err(|e| CliError::InvalidInput(format!("captures JSON parse error: {e}")))?;
            let batch_id = Uuid::new_v4().to_string();
            let recorded = commands.record_ingest_batch_classified(
                &actor_id,
                std::slice::from_ref(&batch_id),
                &args.classifier_model,
                &args.schema_version,
                captures,
                None,
            )?;
            OutputEnvelope::new("emit", "batch_id", batch_id).with_restated(recorded.restated)
        }
        EmitCommand::DecisionScored(args) => {
            let (actor_id, provenance) = capture_actor_and_provenance(&args.provenance)?;
            let commands =
                Commands::new_with_context(&ledger, cli_command_context(cli, provenance)?);
            let tenant_id = cli_tenant(cli)?;
            let (decision_id, causation_event_id) = match (
                &args.decision_id,
                &args.batch_id,
                args.capture_index,
            ) {
                (Some(decision_id), None, None) => (decision_id.clone(), None),
                (None, Some(batch_id), Some(capture_index)) => {
                    let (capture_node_id, batch_event_id) = crate::scorer::resolve_capture_node_id(
                        &ledger,
                        &tenant_id,
                        batch_id,
                        capture_index,
                    )?;
                    (capture_node_id, Some(batch_event_id))
                }
                _ => return Err(CliError::InvalidInput(
                    "exactly one of --decision-id or (--batch-id and --capture-index) is required"
                        .to_owned(),
                )
                .into()),
            };
            let json_text = std::fs::read_to_string(&args.scores_file).map_err(|e| {
                CliError::InvalidInput(format!(
                    "cannot read scores file {:?}: {e}",
                    args.scores_file
                ))
            })?;
            let output: crate::scorer::AssessmentOutput = serde_json::from_str(&json_text)
                .map_err(|e| CliError::InvalidInput(format!("scores JSON parse error: {e}")))?;
            let payload = crate::scorer::build_assessed_payload(
                &decision_id,
                &args.model,
                &args.prompt_version,
                output,
            );
            let event_id =
                commands.record_decision_assessed(&actor_id, payload, causation_event_id)?;
            OutputEnvelope::new("emit", "event_id", event_id.to_string())
        }
    };

    if let Some(placement) = &output.placement {
        announce_placement(cli, placement, output.project_reminder.as_deref(), notices);
    }
    format_output(cli.json, &output)
}

/// Text mode only: say where a capture landed on `notices` (stderr in the CLI). JSON replies
/// carry `project`/`project_source` in-band, and text stdout stays machine-readable (the bare
/// id), so this is the one place a human or agent reading the terminal sees the project --
/// and the "saved to your personal project" sentence on fallback, followed by whatever
/// `--project-from-context` has to say about how it worked the project out: the "this folder is
/// not attached" reminder when it found nothing to attach to, or the note that a change
/// spanning several projects was recorded for their parent or saved to the personal project.
/// Best-effort: the event is already appended, so an unwritable stderr must not turn a
/// recorded capture into an error.
fn announce_placement<W: IoWrite>(
    cli: &Cli,
    placement: &DecisionPlacement,
    reminder: Option<&str>,
    notices: &mut W,
) {
    if !cli.json {
        let _ = writeln!(notices, "{}", render_placement_line(placement));
        if let Some(reminder) = reminder {
            let _ = writeln!(notices, "{reminder}");
        }
    }
}

/// Text mode only: name each decision the rationale cites by id that the capture does not rest
/// on, on `notices` (stderr in the CLI) so the bare id on stdout stays what scripts parse. A hint:
/// the capture is recorded, and `ground` adds the link. JSON replies carry `cited_not_linked`.
fn announce_cited_not_linked<W: IoWrite>(
    cli: &Cli,
    decision_id: &str,
    cited_not_linked: &[String],
    notices: &mut W,
) {
    if cli.json {
        return;
    }
    for cited_id in cited_not_linked {
        let _ = writeln!(
            notices,
            "cited_not_linked: {cited_id} (the rationale names it, but the decision does not rest on it; if it follows from it: hivemind ground --id {decision_id} --rests-on-decision {cited_id})"
        );
    }
}

/// Text mode only: one line per warning on `notices` (stderr in the CLI), so the bare id on stdout
/// stays what scripts parse. The capture is recorded either way. JSON replies carry `warnings`.
fn announce_warnings<W: IoWrite>(cli: &Cli, warnings: &[CaptureWarning], notices: &mut W) {
    if cli.json {
        return;
    }
    for warning in warnings {
        let _ = writeln!(notices, "warning: {}", warning.message);
    }
}

// ---------------------------------------------------------------------------
// Fluent resolve-by-description support (hivemind-tenv.1)
//
// Every fluent verb (disagree, supersede, get_supersession_chain/chain,
// get_decision_neighborhood/why, compact-view, get_decision_outcome/verify) resolves its target
// through `resolve_fluent_target` before doing anything else. `--id` bypasses resolution
// entirely and is unchanged from today's behavior. See docs/AGENT_FLUENT_QUERYING.md.
// ---------------------------------------------------------------------------

/// Outcome of resolving a fluent verb's target: either a concrete decision id to proceed with,
/// or output the caller must return directly (ambiguous/not-found) without proceeding.
enum FluentResolution {
    Id(String),
    Output(String),
}

fn resolve_fluent_target(
    hivemind_dir: &Path,
    summary: bool,
    graph: &impl GraphView,
    id: Option<&str>,
    description: Option<&str>,
    pick: Option<usize>,
    topic: Option<&str>,
) -> Result<FluentResolution> {
    let ask = FluentAsk {
        id,
        description,
        pick,
        topic,
    };
    let (resolution, _) = resolve_fluent(hivemind_dir, summary, graph, ask, FluentVerb::Writes)?;
    Ok(resolution)
}

/// Resolves a read-only verb's target (`why`, `verify`, `chain`, `compact-view`). Besides the
/// resolution it returns the close match the target rests on, when the description named a word
/// the decision lacks and it was answered anyway: the caller labels its output with it
/// (`format_close_matched_response`), so a partial match never reads as a full one.
fn resolve_fluent_target_for_reading(
    hivemind_dir: &Path,
    summary: bool,
    graph: &impl GraphView,
    id: Option<&str>,
    description: Option<&str>,
    pick: Option<usize>,
    topic: Option<&str>,
) -> Result<(FluentResolution, Option<ResolvedCandidate>)> {
    let ask = FluentAsk {
        id,
        description,
        pick,
        topic,
    };
    resolve_fluent(hivemind_dir, summary, graph, ask, FluentVerb::Reads)
}

/// Whether a fluent verb changes the decision it resolves or only shows it. A read-only verb
/// answers with a close match that leads alone; a writing verb never does.
#[derive(Clone, Copy)]
enum FluentVerb {
    Reads,
    Writes,
}

/// The selectors every fluent verb takes: an id, or a description with its `--pick` and `--topic`.
struct FluentAsk<'a> {
    id: Option<&'a str>,
    description: Option<&'a str>,
    pick: Option<usize>,
    topic: Option<&'a str>,
}

fn resolve_fluent(
    hivemind_dir: &Path,
    summary: bool,
    graph: &impl GraphView,
    ask: FluentAsk<'_>,
    verb: FluentVerb,
) -> Result<(FluentResolution, Option<ResolvedCandidate>)> {
    let FluentAsk {
        id,
        description,
        pick,
        topic,
    } = ask;
    // `--id` bypasses resolution entirely, byte-for-byte, including an empty string: existing
    // scripts and their error messages (e.g. an empty-id validation error from the underlying
    // verb) must be unaffected by this fluent addition.
    if let Some(id) = id {
        return Ok((FluentResolution::Id(id.to_owned()), None));
    }

    let Some(description) = description.map(str::trim).filter(|value| !value.is_empty()) else {
        return Err(CliError::InvalidInput(
            "one of --id, a free-text description, or #N is required".to_owned(),
        )
        .into());
    };

    if let Some(index) = candidate_handle_index(description) {
        let candidate = read_continuation_candidate(hivemind_dir, index)?;
        return Ok((FluentResolution::Id(candidate.decision_id), None));
    }

    let response = match verb {
        FluentVerb::Reads => resolve_decision_for_reading(graph, description, topic)?,
        FluentVerb::Writes => resolve_decision_by_description(graph, description, topic)?,
    };
    write_continuation_candidates(hivemind_dir, &candidates_of(&response.data));

    match &response.data {
        ResolveOutcome::Resolved { candidate } => {
            let close_match = candidate.needs_annotation().then(|| candidate.clone());
            Ok((
                FluentResolution::Id(candidate.decision_id.clone()),
                close_match,
            ))
        }
        ResolveOutcome::Ambiguous { candidates } => {
            if let Some(pick) = pick {
                let index = pick
                    .checked_sub(1)
                    .ok_or_else(|| CliError::InvalidInput("--pick must be >= 1".to_owned()))?;
                match candidates.get(index) {
                    Some(candidate) => {
                        Ok((FluentResolution::Id(candidate.decision_id.clone()), None))
                    }
                    None => Err(CliError::InvalidInput(format!(
                        "--pick {pick} is out of range: {} candidate(s) matched",
                        candidates.len()
                    ))
                    .into()),
                }
            } else {
                let output = format_query_response(
                    summary,
                    &response,
                    render_resolve_outcome_summary,
                    None,
                )?;
                Ok((FluentResolution::Output(output), None))
            }
        }
        ResolveOutcome::NotFound => {
            let output =
                format_query_response(summary, &response, render_resolve_outcome_summary, None)?;
            Ok((FluentResolution::Output(output), None))
        }
    }
}

fn candidates_of(outcome: &ResolveOutcome) -> Vec<ResolvedCandidate> {
    match outcome {
        ResolveOutcome::Resolved { candidate } => vec![candidate.clone()],
        ResolveOutcome::Ambiguous { candidates } => candidates.clone(),
        ResolveOutcome::NotFound => Vec::new(),
    }
}

fn candidate_handle_index(description: &str) -> Option<usize> {
    description.strip_prefix('#')?.parse::<usize>().ok()
}

/// CLI-layer UX convenience, not decision memory (AGENTS.md §2): a session-scoped file holding
/// the most recent resolver candidate set so a later `#N` invocation can address it without
/// re-running resolution. Scoped by `$HIVEMIND_SESSION` when set (mayor decision, hivemind-tenv.1,
/// 2026-09-07) so parallel agents sharing one ledger dir don't collide; falls back to a fixed
/// name otherwise.
fn continuation_file_path(hivemind_dir: &Path) -> PathBuf {
    let scope = std::env::var("HIVEMIND_SESSION")
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "default".to_owned());
    let safe_scope: String = scope
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    hivemind_dir.join(format!(".last-candidates-{safe_scope}.json"))
}

/// Best-effort: a write failure here must never block the underlying query or write. This state
/// is UI convenience, never authoritative — losing it just means a future `#N` fails cleanly.
/// Creates `hivemind_dir` lazily: in Postgres mode there is no reason for it to
/// pre-exist (the ledger itself lives in the configured database), but this
/// resolver cache file is always local, so the directory must exist for the
/// write to land.
fn write_continuation_candidates(hivemind_dir: &Path, candidates: &[ResolvedCandidate]) {
    let path = continuation_file_path(hivemind_dir);
    let _ = std::fs::create_dir_all(hivemind_dir);
    if let Ok(json) = serde_json::to_vec(candidates) {
        let _ = std::fs::write(path, json);
    }
}

fn read_continuation_candidate(hivemind_dir: &Path, index: usize) -> Result<ResolvedCandidate> {
    let zero_based = index
        .checked_sub(1)
        .ok_or_else(|| CliError::InvalidInput("#N must be >= 1".to_owned()))?;
    let path = continuation_file_path(hivemind_dir);
    let bytes = std::fs::read(&path).map_err(|error| {
        CliError::InvalidInput(format!(
            "no prior resolver candidates at {}: {error}",
            path.display()
        ))
    })?;
    let candidates: Vec<ResolvedCandidate> = serde_json::from_slice(&bytes).map_err(|error| {
        CliError::InvalidInput(format!("prior resolver candidates are corrupt: {error}"))
    })?;
    candidates.get(zero_based).cloned().ok_or_else(|| {
        CliError::InvalidInput(format!(
            "#{index} is out of range: {} prior candidate(s)",
            candidates.len()
        ))
        .into()
    })
}

fn run_disagree(cli: &Cli, args: &DisagreeArgs) -> Result<String> {
    let tenant_id = cli_tenant(cli)?;
    let ledger = open_ledger(cli)?;

    let graph = MemoryGraph::default();
    rebuild_graph_for_tenant(&ledger, &tenant_id, &graph)?;
    let target = resolve_fluent_target(
        &cli.hivemind_dir,
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

    let commands = Commands::new_with_context(
        &ledger,
        CommandContext::new(tenant_id.clone(), fluent_write_provenance(&cli.actor)),
    );
    let event_id = commands.disagree(&cli.actor, &decision_id, &args.reason)?;
    let after = MemoryGraph::default();
    rebuild_graph_for_tenant(&ledger, &tenant_id, &after)?;
    let decision_status = derive_decision_status(&after, &decision_id)?;
    let upstream = upstream_of(&after, &decision_id, None)?;

    format_disagree_output(
        cli.json,
        &DisagreeCommandOutput {
            decision_id,
            event_id,
            decision_status,
            rests_on: upstream.rests_on,
            raise_upstream: upstream.raise_upstream,
        },
    )
}

fn run_move(cli: &Cli, args: &MoveArgs) -> Result<String> {
    let tenant_id = cli_tenant(cli)?;
    let ledger = open_ledger(cli)?;

    let graph = MemoryGraph::default();
    rebuild_graph_for_tenant(&ledger, &tenant_id, &graph)?;
    let target = resolve_fluent_target(
        &cli.hivemind_dir,
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

    let commands = Commands::new_with_context(
        &ledger,
        CommandContext::new(tenant_id, fluent_write_provenance(&cli.actor)),
    );
    let outcome = commands.move_decision_to(
        &cli.actor,
        &decision_id,
        args.to.trim(),
        args.reason.as_deref(),
    )?;

    format_move_output(cli.json, &outcome)
}

fn run_retitle(cli: &Cli, args: &RetitleArgs) -> Result<String> {
    let tenant_id = cli_tenant(cli)?;
    let ledger = open_ledger(cli)?;

    let graph = MemoryGraph::default();
    rebuild_graph_for_tenant(&ledger, &tenant_id, &graph)?;
    let target = resolve_fluent_target(
        &cli.hivemind_dir,
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

    let commands = Commands::new_with_context(
        &ledger,
        CommandContext::new(tenant_id, fluent_write_provenance(&cli.actor)),
    );
    let outcome = commands.retitle_decision_to(
        &cli.actor,
        &decision_id,
        args.to.trim(),
        args.reason.as_deref(),
    )?;

    format_retitle_output(cli.json, &outcome)
}

fn run_supersede(cli: &Cli, args: &SupersedeArgs) -> Result<String> {
    run_supersede_with_notices(cli, args, &mut io::stderr())
}

/// `run_supersede`, with the text-mode project announcement written to `notices`; see
/// `announce_placement`.
pub(crate) fn run_supersede_with_notices<W: IoWrite>(
    cli: &Cli,
    args: &SupersedeArgs,
    notices: &mut W,
) -> Result<String> {
    run_supersede_in_context(
        cli,
        args,
        notices,
        &ProjectContextEnv::from_process(&cli.person()),
    )
}

/// `run_supersede_with_notices` with the `--project-from-context` surroundings supplied; see
/// `run_emit_in_context`.
pub(crate) fn run_supersede_in_context<W: IoWrite>(
    cli: &Cli,
    args: &SupersedeArgs,
    notices: &mut W,
    env: &ProjectContextEnv,
) -> Result<String> {
    let tenant_id = cli_tenant(cli)?;
    let ledger = open_ledger(cli)?;

    // The grounding flags are read (and an empty grounding refused) before the old decision is
    // resolved: resolving a description rewrites the `#N` continuation file a premise may point
    // at, and a supersede that names nothing it rests on should fail before doing any work.
    let grounding_spec = grounding::require_grounding(grounding::grounding_spec_from_args(
        &cli.hivemind_dir,
        &args.grounding,
        &args.evidence_ids,
        &args.hypothesis_ids,
    )?)?;
    let floor_answers = floor::floor_answers_from_args(&args.floor)?;

    let graph = MemoryGraph::default();
    rebuild_graph_for_tenant(&ledger, &tenant_id, &graph)?;
    let target = resolve_fluent_target(
        &cli.hivemind_dir,
        !cli.json,
        &graph,
        args.old_decision_id.as_deref(),
        args.description.as_deref(),
        args.pick,
        args.topic.as_deref(),
    )?;
    let old_decision_id = match target {
        FluentResolution::Id(decision_id) => decision_id,
        FluentResolution::Output(output) => return Ok(output),
    };

    let resolved = grounding::resolve_declared_grounding(
        &cli.hivemind_dir,
        &ledger,
        &tenant_id,
        grounding_spec,
    )?;

    let commands = Commands::new_with_context(
        &ledger,
        CommandContext::new(tenant_id.clone(), fluent_write_provenance(&cli.actor)),
    )
    .declaring_topics(&args.declare_topics);
    // A context that finds nothing leaves the project unstated, and the superseding decision
    // keeps inheriting the old decision's project rather than dropping to a personal one.
    let project = cli_supersede_project(cli, &ledger, env, args)?;
    let option_labels = args.options.labels();
    let outcome = commands.supersede(SupersedeInput {
        project: determined_project(&project),
        actor_id: &cli.actor,
        old_decision_id: &old_decision_id,
        new_title: &args.title,
        new_rationale: &args.rationale,
        topic_keys: &args.topic_keys,
        option_labels: &option_labels,
        chosen_option_label: args.chosen_option_label.as_deref(),
        still_proposed: args.still_proposed,
        // The plan carries every id; see `SupersedeInput::grounding`.
        hypothesis_ids: &[],
        evidence_ids: &[],
        grounding: Some(&resolved.plan),
        expressed_confidence: args.grounding.confidence.as_deref(),
        floor: FloorInput::Stated(&floor_answers),
    })?;
    let floor_reply = floor_answers.reply();
    let after = MemoryGraph::default();
    rebuild_graph_for_tenant(&ledger, &tenant_id, &after)?;
    let old_decision_status = derive_decision_status(&after, &old_decision_id)?;
    let new_decision_status = derive_decision_status(&after, &outcome.new_decision_id)?;
    let upstream = upstream_of(&after, &old_decision_id, Some(&outcome.new_decision_id))?;

    let reminder = project_reminder(&project, &outcome.placement);
    announce_placement(cli, &outcome.placement, reminder.as_deref(), notices);
    floor::announce_below_floor(cli, &floor_reply, notices);
    announce_warnings(cli, &outcome.warnings, notices);
    format_supersede_output(
        cli.json,
        &SupersedeCommandOutput {
            old_decision_id,
            new_decision_id: outcome.new_decision_id,
            proposal_event_id: outcome.proposal_event_id,
            relation_event_ids: outcome.relation_event_ids,
            superseded_event_id: outcome.superseded_event_id,
            old_decision_status,
            new_decision_status,
            project_notice: outcome.placement.notice(),
            project_reminder: reminder,
            placement: outcome.placement,
            rests_on: resolved.label(outcome.rests_on),
            premise_stale: outcome.premise_stale,
            warnings: outcome.warnings,
            raise_upstream: upstream.raise_upstream,
            below_floor: floor_reply.below_floor(),
            below_floor_note: floor_reply.note(),
            floor: floor_reply,
        },
    )
}

fn run_review(cli: &Cli, args: &ReviewArgs) -> Result<String> {
    let stdin = io::stdin();
    let mut input = stdin.lock();
    let stderr = io::stderr();
    let mut prompt_output = stderr.lock();
    run_review_session(cli, args, &mut input, &mut prompt_output)
}

pub(crate) fn run_review_session<R: BufRead, W: IoWrite>(
    cli: &Cli,
    args: &ReviewArgs,
    input: &mut R,
    prompt_output: &mut W,
) -> Result<String> {
    let tenant_id = cli_tenant(cli)?;
    let ledger = open_ledger(cli)?;
    let scoped_ledger = TenantScopedLedger::new(&ledger, tenant_id.clone());
    let request = review_recent_decisions_request(args)?;
    let graph = MemoryGraph::default();
    rebuild_graph_for_tenant(&ledger, &tenant_id, &graph)?;
    let response = get_recent_decisions(&scoped_ledger, &graph, &request)?;
    let events = read_ledger_events(&scoped_ledger)?;
    let context = ReviewLedgerContext::from_events(&events)?;
    let reviewed_decision_ids = reviewed_decision_ids_by_actor(&events, &cli.actor)?;
    let mut items = response.data.items;

    if args.unreviewed_only {
        items.retain(|item| !reviewed_decision_ids.contains(&item.decision_id));
    }

    if items.is_empty() {
        writeln!(prompt_output, "No matching decisions to review.").map_err(cli_io_error)?;
        return format_review_output(
            cli.json,
            &ReviewCommandOutput {
                reviewer_actor_id: cli.actor.clone(),
                matched_count: 0,
                reviewed_count: 0,
                skipped_count: 0,
                quit: false,
                truncated: response.truncated,
                next_cursor: response.data.next_cursor,
                unreviewed_only: args.unreviewed_only,
                reviewed_semantics: REVIEWED_SEMANTICS,
                actions: Vec::new(),
            },
        );
    }

    let commands = Commands::new_with_context(
        &ledger,
        CommandContext::new(tenant_id.clone(), fluent_write_provenance(&cli.actor)),
    );
    let mut actions = Vec::new();
    let mut quit = false;
    let matched_count = items.len();

    for (index, item) in items.into_iter().enumerate() {
        render_review_item(prompt_output, index + 1, matched_count, &item, &context)?;

        loop {
            let Some(action) = prompt_line(
                input,
                prompt_output,
                "Action [a approve, d disagree, s supersede, n next, q quit]: ",
            )?
            else {
                quit = true;
                break;
            };
            match action.trim().to_ascii_lowercase().as_str() {
                "a" | "approve" => {
                    let event_id = commands.accept_decision(&item.decision_id, &cli.actor)?;
                    let status =
                        decision_status_after_write(&ledger, &tenant_id, &item.decision_id)?;
                    actions.push(ReviewActionOutput {
                        decision_id: item.decision_id,
                        action: "approved",
                        event_id: Some(event_id),
                        proposal_event_id: None,
                        superseded_event_id: None,
                        new_decision_id: None,
                        old_decision_status: Some(status),
                        new_decision_status: None,
                    });
                    break;
                }
                "d" | "disagree" => {
                    let Some(reason) = prompt_required_line(
                        input,
                        prompt_output,
                        "Disagreement reason: ",
                        "reason must not be empty",
                    )?
                    else {
                        quit = true;
                        break;
                    };
                    let event_id = commands.disagree(&cli.actor, &item.decision_id, &reason)?;
                    let status =
                        decision_status_after_write(&ledger, &tenant_id, &item.decision_id)?;
                    actions.push(ReviewActionOutput {
                        decision_id: item.decision_id,
                        action: "disagreed",
                        event_id: Some(event_id),
                        proposal_event_id: None,
                        superseded_event_id: None,
                        new_decision_id: None,
                        old_decision_status: Some(status),
                        new_decision_status: None,
                    });
                    break;
                }
                "s" | "supersede" => {
                    let Some(title) = prompt_required_line(
                        input,
                        prompt_output,
                        "New decision title: ",
                        "title must not be empty",
                    )?
                    else {
                        quit = true;
                        break;
                    };
                    let Some(rationale) = prompt_required_line(
                        input,
                        prompt_output,
                        "New decision rationale: ",
                        "rationale must not be empty",
                    )?
                    else {
                        quit = true;
                        break;
                    };
                    let option_labels = prompt_line(
                        input,
                        prompt_output,
                        "New option labels, comma-separated (blank for default): ",
                    )?
                    .map(|line| split_review_list(&line))
                    .unwrap_or_default();
                    let chosen_option_label = prompt_line(
                        input,
                        prompt_output,
                        "Chosen option label (blank for none): ",
                    )?
                    .and_then(|line| non_empty_owned(&line));

                    let outcome = commands.supersede(SupersedeInput {
                        project: None,
                        actor_id: &cli.actor,
                        old_decision_id: &item.decision_id,
                        new_title: &title,
                        new_rationale: &rationale,
                        topic_keys: &item.topic_keys,
                        option_labels: &option_labels,
                        chosen_option_label: chosen_option_label.as_deref(),
                        still_proposed: false,
                        hypothesis_ids: &item.hypothesis_ids,
                        evidence_ids: &item.evidence_ids,
                        grounding: None,
                        expressed_confidence: None,
                        floor: FloorInput::NotAsked,
                    })?;
                    let old_status =
                        decision_status_after_write(&ledger, &tenant_id, &item.decision_id)?;
                    let new_status =
                        decision_status_after_write(&ledger, &tenant_id, &outcome.new_decision_id)?;
                    actions.push(ReviewActionOutput {
                        decision_id: item.decision_id,
                        action: "superseded",
                        event_id: None,
                        proposal_event_id: Some(outcome.proposal_event_id),
                        superseded_event_id: Some(outcome.superseded_event_id),
                        new_decision_id: Some(outcome.new_decision_id),
                        old_decision_status: Some(old_status),
                        new_decision_status: Some(new_status),
                    });
                    break;
                }
                "" | "n" | "next" | "skip" => {
                    actions.push(ReviewActionOutput {
                        decision_id: item.decision_id,
                        action: "skipped",
                        event_id: None,
                        proposal_event_id: None,
                        superseded_event_id: None,
                        new_decision_id: None,
                        old_decision_status: Some(item.status),
                        new_decision_status: None,
                    });
                    break;
                }
                "q" | "quit" => {
                    quit = true;
                    break;
                }
                other => {
                    writeln!(
                        prompt_output,
                        "Unknown action '{other}'. Use a, d, s, n, or q."
                    )
                    .map_err(cli_io_error)?;
                }
            }
        }

        if quit {
            break;
        }
    }

    let reviewed_count = actions
        .iter()
        .filter(|action| action.action != "skipped")
        .count();
    let skipped_count = actions
        .iter()
        .filter(|action| action.action == "skipped")
        .count();

    format_review_output(
        cli.json,
        &ReviewCommandOutput {
            reviewer_actor_id: cli.actor.clone(),
            matched_count,
            reviewed_count,
            skipped_count,
            quit,
            truncated: response.truncated,
            next_cursor: response.data.next_cursor,
            unreviewed_only: args.unreviewed_only,
            reviewed_semantics: REVIEWED_SEMANTICS,
            actions,
        },
    )
}

/// The prose extractor the flags name, if any. Clap refuses `--extractor-command` beside
/// `--llm-response`, and `--extractor-arg` without `--extractor-command` or beside
/// `--llm-response`, while parsing, before the ledger is opened (hivemind-f9l9y), so at most one
/// source is set here.
fn build_import_extractor(args: &ImportDocumentsArgs) -> Option<DocumentCandidateExtractor> {
    match (&args.extractor_command, &args.llm_response) {
        (Some(_), _) => Some(DocumentCandidateExtractor::Command {
            args: args.extractor_args.clone(),
        }),
        (None, Some(path)) => Some(DocumentCandidateExtractor::ResponseFile(path.clone())),
        (None, None) => None,
    }
}

fn apply_prose_extraction<L: crate::ledger::EventLedger>(
    ledger: &L,
    report: &mut crate::ingest::DocumentImportReport,
    format: DocumentImportFormat,
    extractor: &DocumentCandidateExtractor,
    importer_actor_id: &str,
) -> Result<()> {
    let prose_indices: Vec<usize> = report
        .files
        .iter()
        .enumerate()
        .filter(|(_, f)| f.status == DocumentFileImportStatus::SkippedUnmarked)
        .map(|(i, _)| i)
        .collect();

    if prose_indices.is_empty() {
        return Ok(());
    }

    let prose_paths: Vec<std::path::PathBuf> = prose_indices
        .iter()
        .filter_map(|&i| {
            report.files[i] // ubs:ignore: idx from enumerate of report.files — always in bounds
                .canonical_path
                .as_deref()
                .map(std::path::PathBuf::from)
        })
        .collect();

    if prose_paths.is_empty() {
        return Ok(());
    }

    let extraction = propose_document_extraction_candidates(&DocumentCandidateRequest {
        paths: prose_paths,
        format,
        extractor: extractor.clone(),
    })?;

    // Index candidates by canonical source path
    let mut candidates_by_canonical: HashMap<String, Vec<_>> = HashMap::new();
    for candidate in extraction.candidates {
        candidates_by_canonical
            .entry(candidate.source.path.clone()) // ubs:ignore: path moved as entry key; candidate pushed whole after
            .or_default()
            .push(candidate);
    }

    // Index extraction source files by canonical_path → sha256 and display path
    let source_by_canonical: HashMap<String, &DocumentExtractionSourceDocument> = extraction
        .files
        .iter()
        .map(|f| (f.canonical_path.clone(), f)) // ubs:ignore: f is &ref; clone required for HashMap key
        .collect();

    let import_run_id = report.import_run_id.clone();

    for &idx in &prose_indices {
        let file = &report.files[idx]; // ubs:ignore: idx from enumerate of report.files — always in bounds
        let canonical = match file.canonical_path.as_deref() {
            Some(c) => c.to_owned(), // ubs:ignore: &str from as_deref; owned String needed as HashMap key
            None => continue,
        };
        let src = match source_by_canonical.get(&canonical) {
            Some(s) => *s,
            None => continue,
        };
        let raw_candidates = candidates_by_canonical
            .remove(&canonical)
            .unwrap_or_default();
        let prose_candidates: Vec<ProseImportCandidate> = raw_candidates
            .into_iter()
            .map(|c| ProseImportCandidate {
                candidate_id: c.candidate_id,
                title: c.decision.title,
                status_str: c.decision.status,
                topic_keys: c.decision.topic_keys,
                rationale: c.decision.rationale,
                option_labels: c.decision.options,
                chosen_option_label: c.decision.chose,
                evidence: c.decision.evidence,
                hypotheses: c.decision.hypotheses,
                source_span: c.source.span,
                source_snippet: c.source.snippet,
                explanation: c.explanation,
            })
            .collect();

        let prose_source = ProseImportSource {
            path: report.files[idx].path.clone(), // ubs:ignore: idx in bounds; clone needed — report.files still borrowed below
            canonical_path: canonical.clone(), // ubs:ignore: canonical reused for candidates_by_canonical.remove above
            sha256: src.sha256.clone(), // ubs:ignore: src is &ref; clone required for owned field
        };

        let prose_report = import_prose_file_candidates(
            ledger,
            &prose_source,
            prose_candidates,
            importer_actor_id,
            &import_run_id,
        )?;
        report.files[idx] = prose_report; // ubs:ignore: idx from enumerate of report.files — always in bounds
    }

    // Re-compute summary over updated file reports
    let mut new_summary = DocumentImportSummary {
        files_seen: report.summary.files_seen,
        ..DocumentImportSummary::default()
    };
    for file in &report.files {
        accumulate_file_summary_pub(&mut new_summary, file);
    }
    report.summary = new_summary;

    Ok(())
}

fn run_import(cli: &Cli, import: &ImportArgs) -> Result<String> {
    match &import.command {
        ImportCommand::Documents(args) => {
            let ledger = open_ledger(cli)?;
            let scoped_ledger = TenantScopedLedger::new(&ledger, cli_tenant(cli)?);
            let mut paths = args.files.clone();
            paths.extend(args.paths.clone());
            let mut report = import_documents(
                &scoped_ledger,
                &DocumentImportRequest {
                    paths,
                    importer_actor_id: cli.actor.clone(),
                    format: args.format.as_ingest_format(),
                    conflict_resolution: args.on_conflict.as_ingest_action(),
                },
            )?;
            if let Some(extractor) = build_import_extractor(args) {
                apply_prose_extraction(
                    &scoped_ledger,
                    &mut report,
                    args.format.as_ingest_format(),
                    &extractor,
                    &cli.actor,
                )?;
            }
            format_import_output(cli.json, &report)
        }
        ImportCommand::PrepareDocuments(args) => {
            let mut paths = args.files.clone();
            paths.extend(args.paths.clone());
            let report = prepare_document_texts(&DocumentPreparationRequest {
                paths,
                format: args.format.as_ingest_format(),
                output_dir: args.output_dir.clone(),
            })?;
            format_prepare_documents_output(cli.json, &report)
        }
        ImportCommand::Connector(args) => {
            use crate::connector::{
                confirm_same_as, find_connector_same_as_candidates, import_via_connector,
                retract_same_as, ConnectorImportRequest, GitFileConnector, GoogleDocsConnector,
                SameAsConfig,
            };
            let ledger = open_ledger(cli)?;
            let tenant_id = cli_tenant(cli)?;
            match &args.command {
                ImportConnectorCommand::Run(run_args) => {
                    let mut connectors: Vec<Box<dyn crate::connector::Connector>> =
                        vec![Box::new(GitFileConnector)];
                    if let Ok(Some(gdocs)) = GoogleDocsConnector::new(&cli.hivemind_dir) {
                        connectors.push(Box::new(gdocs));
                    }
                    let report = import_via_connector(
                        &ledger,
                        &tenant_id,
                        &ConnectorImportRequest {
                            url_or_id: run_args.url_or_id.clone(), // ubs:ignore: clone required to own from &ImportConnectorRunArgs
                            importer_actor_id: cli.actor.clone(), // ubs:ignore: clone required to own from &Cli
                            max_versions: run_args.max_versions,
                            import_run_id: None,
                        },
                        &connectors,
                    )?;
                    format_json_value(cli.json, &report)
                }
                ImportConnectorCommand::SameAsCandidates(ca) => {
                    let report = find_connector_same_as_candidates(
                        &ledger,
                        &tenant_id,
                        &ca.import_run_id,
                        &SameAsConfig::default(),
                    )?;
                    format_json_value(cli.json, &report)
                }
                ImportConnectorCommand::ConfirmSameAs(ca) => {
                    let report = confirm_same_as(
                        &ledger,
                        &tenant_id,
                        &ca.left_id,
                        &ca.right_id,
                        &cli.actor,
                    )?;
                    format_json_value(cli.json, &report)
                }
                ImportConnectorCommand::RetractSameAs(ra) => {
                    let report = retract_same_as(
                        &ledger,
                        &tenant_id,
                        &ra.left_id,
                        &ra.right_id,
                        &cli.actor,
                    )?;
                    format_json_value(cli.json, &report)
                }
            }
        }
    }
}

/// Record the CLI's `--options` values and resolve `--chose` among them. Options live only in
/// the `Commands` instance until a decision proposes them, so this writes nothing to the ledger.
fn record_cli_options<L: EventLedger>(
    commands: &Commands<'_, L>,
    actor_id: &str,
    option_labels: &[String],
    chosen_option_label: Option<&str>,
) -> Result<(Vec<String>, Option<String>)> {
    let mut option_ids = Vec::with_capacity(option_labels.len());
    let mut chosen_option_id = None;
    for option_label in option_labels {
        let mut option_description =
            String::with_capacity("Option generated from CLI value ''".len() + option_label.len());
        let _ = write!(
            option_description,
            "Option generated from CLI value '{option_label}'"
        );
        let option_id = commands.record_option(actor_id, option_label, &option_description)?;
        if chosen_option_label == Some(option_label.as_str()) {
            chosen_option_id = Some(option_id.clone());
        }
        option_ids.push(option_id);
    }

    if chosen_option_label.is_some() && chosen_option_id.is_none() {
        return Err(CliError::InvalidInput(
            "--chose must match one of the labels passed to --options or --option".to_owned(),
        )
        .into());
    }

    Ok((option_ids, chosen_option_id))
}

fn propose_decision_from_option_labels<L: EventLedger>(
    commands: &Commands<'_, L>,
    actor_id: &str,
    args: &EmitDecisionProposedArgs,
    project: Option<DeterminedProject<'_>>,
) -> Result<(String, DecisionPlacement)> {
    let option_labels = args.options.labels();
    let (option_ids, chosen_option_id) = record_cli_options(
        commands,
        actor_id,
        &option_labels,
        args.chosen_option_id.as_deref(),
    )?;

    commands.propose_decision_placed(DecisionProposalInput {
        project,
        actor_id,
        title: &args.title,
        rationale: &args.rationale,
        topic_keys: &args.topic_keys,
        option_ids: &option_ids,
        option_labels: &option_labels,
        chosen_option_id: chosen_option_id.as_deref(),
        decided_by: args.decided_by.as_deref(),
        delegated_by: args.delegated_by.as_deref(),
        still_proposed: args.still_proposed,
        hypothesis_ids: &args.hypothesis_ids,
        evidence_ids: &args.evidence_ids,
        quote: args.quote.as_deref(),
        question: args.question.as_deref(),
        // Raw `emit decision.proposed` never asks the capture question; `decision.capture`
        // goes through `propose_grounded_decision` instead (hivemind-gwhr.2).
        grounding: Grounding::NotAsked,
        expressed_confidence: None,
        floor: FloorInput::NotAsked,
    })
}

fn emit_actor_and_commands<'a>(
    cli: &Cli,
    ledger: &'a AnyLedger,
    provenance_args: &EmitCaptureProvenanceArgs,
) -> Result<(String, Commands<'a, AnyLedger>)> {
    if !provenance_args.has_override() {
        let commands = Commands::new_with_context(
            ledger,
            cli_command_context(cli, cli_emit_provenance(&cli.actor))?,
        );
        return Ok((cli.actor.clone(), commands));
    }

    let (actor_id, provenance) = capture_actor_and_provenance(provenance_args)?;
    let commands = Commands::new_with_context(ledger, cli_command_context(cli, provenance)?);
    Ok((actor_id, commands))
}

/// Who `emit decision.capture` records. It is an agent surface, so the agent derivation below
/// is the default; but it must not overwrite a person. With no provenance flag given, `cli.actor`
/// is recorded, with the source its shape implies (`human:` a person, `agent:` an agent), when
///
/// - `--actor` was typed: a caller who says who is acting is believed (README: `--actor`
///   overrides the actor), or
/// - the environment shows no agent at all (`agent_present` is false): the person at a plain
///   terminal is the CLI's default actor, not an invented `agent:unknown:manual-session`.
///
/// The provenance flags (`--source`, `--actor-id`, `--agent-tool`, ...) are the fuller form the
/// capture plugins pass, and still win.
pub(crate) fn decision_capture_actor_and_provenance(
    cli: &Cli,
    args: &EmitCaptureProvenanceArgs,
    agent_present: bool,
) -> Result<(String, EventProvenance)> {
    if !args.has_override() && (cli.actor_given.0 || !agent_present) {
        let actor_id = trimmed_required("--actor", &cli.actor)?.to_owned();
        let provenance = fluent_write_provenance(&actor_id);
        return Ok((actor_id, provenance));
    }
    capture_actor_and_provenance(args)
}

fn capture_actor_and_provenance(
    args: &EmitCaptureProvenanceArgs,
) -> Result<(String, EventProvenance)> {
    let actor_id = capture_actor_id(args)?;
    let provenance = capture_provenance(args, &actor_id)?;
    Ok((actor_id, provenance))
}

fn capture_actor_id(args: &EmitCaptureProvenanceArgs) -> Result<String> {
    if let Some(actor_id) = trimmed_optional("--actor-id", &args.actor_id)? {
        return Ok(actor_id.to_owned());
    }

    match args.source.unwrap_or(DecisionCaptureSource::Agent) {
        DecisionCaptureSource::Agent => {
            let tool = capture_agent_tool(args)?;
            let session = capture_agent_session(args, &tool)?;
            Ok(agent_actor_id(&tool, &session))
        }
        DecisionCaptureSource::Human => Ok(default_human_actor_id()),
    }
}

fn capture_agent_tool(args: &EmitCaptureProvenanceArgs) -> Result<String> {
    trimmed_optional("--agent-tool", &args.agent_tool).map(|value| {
        value
            .map(ToOwned::to_owned)
            .unwrap_or_else(default_agent_tool)
    })
}

fn capture_agent_session(args: &EmitCaptureProvenanceArgs, tool: &str) -> Result<String> {
    trimmed_optional("--agent-session", &args.agent_session).map(|value| {
        value
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| default_agent_session(tool))
    })
}

fn capture_provenance(args: &EmitCaptureProvenanceArgs, actor_id: &str) -> Result<EventProvenance> {
    let source = args.source.unwrap_or(DecisionCaptureSource::Agent);
    if let Some(source_ref) = trimmed_optional("--source-ref", &args.source_ref)? {
        return Ok(match source {
            DecisionCaptureSource::Agent => EventProvenance::agent(source_ref),
            DecisionCaptureSource::Human => EventProvenance::human(source_ref),
        });
    }

    Ok(match source {
        // The raw per-run session id (never the stable name `actor_id` now carries --
        // `agent_session_from_env` prefers a stable Gas City identity over it, see
        // identity.rs) is exactly what provenance is for: it lets a specific capture's
        // run still be traced even though actor_id no longer varies per run
        // (hivemind-zdsh.9). Only applies to ambient derivation, though: when the caller
        // pinned the session or actor explicitly, that pin is itself the intended claim
        // for this capture and stays authoritative in source_ref too, matching actor_id
        // (the prior, pre-zdsh.9 default).
        DecisionCaptureSource::Agent => {
            let explicit = trimmed_optional("--agent-session", &args.agent_session)?.is_some()
                || trimmed_optional("--actor-id", &args.actor_id)?.is_some();
            let source_ref = if explicit {
                actor_id.to_owned()
            } else {
                let tool = capture_agent_tool(args)?;
                raw_agent_session_from_env(&tool).unwrap_or_else(|| actor_id.to_owned())
            };
            EventProvenance::agent(source_ref)
        }
        DecisionCaptureSource::Human => EventProvenance::human(actor_id),
    })
}

fn cli_emit_provenance(actor_id: &str) -> EventProvenance {
    if actor_id.trim().starts_with("human:") {
        EventProvenance::human(actor_id.trim().to_owned())
    } else {
        EventProvenance::cli()
    }
}

/// Provenance for the fluent write verbs (disagree, supersede): derives source from
/// `--actor`'s `agent:`/`human:` shape (identity.rs's `agent_actor_id`/
/// `default_human_actor_id` convention), the same way decision.capture already
/// distinguishes source by actor (`capture_provenance`). These verbs previously
/// hardcoded `EventProvenance::human` unconditionally, misattributing every
/// agent-initiated disagree/supersede as human-sourced (AGENTS.md #2, hivemind-xm93).
/// Anything not explicitly agent-shaped defaults to human, preserving prior behavior
/// for bare/human actor ids.
fn fluent_write_provenance(actor_id: &str) -> EventProvenance {
    let trimmed = actor_id.trim();
    if trimmed.starts_with("agent:") {
        EventProvenance::agent(trimmed.to_owned())
    } else {
        EventProvenance::human(trimmed.to_owned())
    }
}

fn trimmed_required<'a>(field: &'static str, value: &'a str) -> Result<&'a str> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        Err(CliError::InvalidInput(format!("{field} must not be empty")).into())
    } else {
        Ok(trimmed)
    }
}

fn trimmed_optional<'a>(field: &'static str, value: &'a Option<String>) -> Result<Option<&'a str>> {
    match value.as_deref() {
        Some(raw) => Ok(Some(trimmed_required(field, raw)?)),
        None => Ok(None),
    }
}

/// `--project`/`--project-source` as the write layer's `DeterminedProject`. The handle is
/// whatever the caller determined; HiveMind only validates it, never infers one.
fn cli_determined_project<'a>(
    project: &'a Option<String>,
    project_source: Option<ProjectSourceArg>,
) -> Result<Option<DeterminedProject<'a>>> {
    Ok(
        trimmed_optional("--project", project)?.map(|handle| DeterminedProject {
            handle,
            source: project_source
                .map_or(ProjectSource::Stated, ProjectSourceArg::as_project_source),
        }),
    )
}

/// What a capture's `--project` / `--project-source` / `--project-from-context` settle on.
///
/// `None` means the caller stated no project and did not ask for one to be worked out: the
/// write layer records the plain personal fallback, with no reminder about the folder. With
/// `--project-from-context` the answer is always `Some`, from the ladder in
/// `super::project_context` (a stated `--project` still wins it).
fn cli_project_from_flags(
    cli: &Cli,
    ledger: &AnyLedger,
    env: &ProjectContextEnv,
    project: &Option<String>,
    project_source: Option<ProjectSourceArg>,
    from_context: bool,
) -> Result<Option<ResolvedProject>> {
    let stated = cli_determined_project(project, project_source)?;
    if !from_context {
        return Ok(stated.map(ResolvedProject::from));
    }
    resolve_project_in_ledger(
        stated,
        env,
        ledger,
        &LedgerConfig::from_cli(cli),
        &cli_tenant(cli)?,
    )
    .map(Some)
}

/// A capture's project, refused when working it out from context found a rig this ledger
/// has no project for (`ResolvedProject::wrong_ledger_refusal`): filing it would put a
/// decision from one place into a ledger that does not know it.
fn cli_capture_project(
    cli: &Cli,
    ledger: &AnyLedger,
    env: &ProjectContextEnv,
    args: &EmitDecisionProposedArgs,
) -> Result<Option<ResolvedProject>> {
    let project = cli_project_from_flags(
        cli,
        ledger,
        env,
        &args.project,
        args.project_source,
        args.project_from_context,
    )?;
    if let Some(refusal) = project
        .as_ref()
        .and_then(ResolvedProject::wrong_ledger_refusal)
    {
        return Err(CliError::InvalidInput(refusal).into());
    }
    Ok(project)
}

fn cli_supersede_project(
    cli: &Cli,
    ledger: &AnyLedger,
    env: &ProjectContextEnv,
    args: &SupersedeArgs,
) -> Result<Option<ResolvedProject>> {
    cli_project_from_flags(
        cli,
        ledger,
        env,
        &args.project,
        args.project_source,
        args.project_from_context,
    )
}

/// The write layer's input for a settled project; `None` when there is nothing to pass.
fn determined_project(project: &Option<ResolvedProject>) -> Option<DeterminedProject<'_>> {
    project.as_ref().and_then(ResolvedProject::determined)
}

/// What `--project-from-context` has to say about how a capture's project was worked out,
/// for its reply: the "this folder is not attached" reminder, or the note about a change that
/// spans several projects. A fallback reminder is only said when the capture really did land
/// in the personal project (a superseding decision that inherited a shared project has
/// nothing to be reminded about).
fn project_reminder(
    project: &Option<ResolvedProject>,
    placement: &DecisionPlacement,
) -> Option<String> {
    project.as_ref()?.reminder(placement.notice().is_some())
}

fn run_query(cli: &Cli, query: &QueryArgs) -> Result<String> {
    let context = cli_query_context(cli)?;
    let ledger = open_ledger(cli)?;

    if query.command.is_ledger_history_query() {
        let scoped_ledger = TenantScopedLedger::new(&ledger, context.tenant_id.clone());
        return run_query_with_ledger(&scoped_ledger, query);
    }

    match selected_graph_backend(cli)? {
        GraphBackend::Memory => {
            let graph = MemoryGraph::default();
            rebuild_graph_for_tenant(&ledger, &context.tenant_id, &graph)?;
            run_query_with_graph(&context, &ledger, &graph, &cli.hivemind_dir, query)
        }
        GraphBackend::Kuzu => run_query_with_kuzu(&context, &ledger, &cli.hivemind_dir, query),
    }
}

impl QueryCommand {
    fn is_ledger_history_query(&self) -> bool {
        matches!(
            self,
            QueryCommand::GetRecentActivity(_)
                | QueryCommand::GetDecisionsChangedSince(_)
                | QueryCommand::ExportReadOnlySummary(_)
        )
    }
}

fn run_query_with_ledger(ledger: &impl EventLedger, query: &QueryArgs) -> Result<String> {
    let output = match &query.command {
        QueryCommand::GetRecentActivity(args) => {
            let response = get_recent_activity(ledger, &recent_activity_request(args)?)?;
            format_query_response(
                query.summary,
                &response,
                render_recent_activity_summary,
                response.data.next_cursor.as_deref(),
            )?
        }
        QueryCommand::GetDecisionsChangedSince(args) => {
            let response = get_decisions_changed_since(ledger, &changed_since_request(args)?)?;
            format_query_response(
                query.summary,
                &response,
                render_changed_since_summary,
                response.data.next_cursor.as_deref(),
            )?
        }
        QueryCommand::ExportReadOnlySummary(args) => {
            let request = export_read_only_summary_request(args)?;
            let response = export_read_only_summary(ledger, &request)?;
            format_query_response(
                query.summary,
                &response,
                render_read_only_export_summary,
                response.data.continuation_cursor.as_deref(),
            )?
        }
        QueryCommand::GetDecision(_)
        | QueryCommand::GetRelevantDecisions(_)
        | QueryCommand::GetSupersessionChain(_)
        | QueryCommand::GetDecisionNeighborhood(_)
        | QueryCommand::GetCompactView(_)
        | QueryCommand::GetDecisionOutcome(_)
        | QueryCommand::RecentDecisions(_)
        | QueryCommand::GetDecisionsAddedSince(_)
        | QueryCommand::Search(_)
        | QueryCommand::SearchDecisions(_)
        | QueryCommand::Recall(_)
        | QueryCommand::GetActiveDecisionBlockers(_)
        | QueryCommand::GetWaitingRequests(_)
        | QueryCommand::GetContestedDecisions(_)
        | QueryCommand::GetOpenWhyDecisions(_)
        | QueryCommand::GetChangedDecisions(_)
        | QueryCommand::GetBlockerNotificationCandidates(_)
        | QueryCommand::ScoreDecision(_)
        | QueryCommand::ScanDecisionQuality(_)
        | QueryCommand::GetSuggestions(_)
        | QueryCommand::RankDecisionsByImportance(_)
        | QueryCommand::ScanMisfiledDecisions(_)
        | QueryCommand::GetSituationalDecisions(_) => {
            return Err(
                CliError::InvalidInput("query requires graph-backed execution".to_owned()).into(),
            )
        }
    };

    Ok(output)
}

const REVIEWED_SEMANTICS: &str =
    "derived from reviewer-authored decision.accepted, decision.rejected, or decision.superseded events";

pub(crate) fn review_recent_decisions_request(args: &ReviewArgs) -> Result<RecentDecisionsRequest> {
    let now = parse_utc_timestamp("--now", &args.now)?;
    let timezone = TimeZoneSpec::parse(&args.timezone)?;
    let since_timestamp =
        resolve_diff_bound("--since", Some(args.since.as_str()), None, now, timezone)?
            .ok_or_else(|| CliError::InvalidInput("--since must not be empty".to_owned()))?;
    let until_timestamp =
        resolve_diff_bound("--until", args.until.as_deref(), None, now, timezone)?;

    Ok(RecentDecisionsRequest {
        since_timestamp,
        until_timestamp,
        filters: RecentDecisionFilterRequest {
            actor_patterns: args.actor_patterns.clone(),
            sources: Vec::new(),
            topic_keys: Vec::new(),
            statuses: Vec::new(),
        },
        limit: args.limit,
        cursor: args.cursor.clone(),
    })
}

fn read_ledger_events(ledger: &impl EventLedger) -> Result<Vec<Event>> {
    let mut events = Vec::new();
    ledger.replay_from(0, &mut |event| {
        // An annotation row nobody can parse is no reason to refuse a review of the decisions.
        if !crate::events::is_unreadable_annotation(event) {
            events.push(event.clone());
        }
        Ok(())
    })?;
    Ok(events)
}

fn reviewed_decision_ids_by_actor(
    events: &[Event],
    reviewer_actor_id: &str,
) -> Result<BTreeSet<String>> {
    let mut reviewed = BTreeSet::new();
    for event in events
        .iter()
        .filter(|event| event.actor_id == reviewer_actor_id)
    {
        match validated_payload(event)? {
            EventPayload::DecisionAccepted(payload) => {
                reviewed.insert(payload.decision_id);
            }
            EventPayload::DecisionRejected(payload) => {
                reviewed.insert(payload.decision_id);
            }
            EventPayload::DecisionSuperseded(payload) => {
                reviewed.insert(payload.old_decision_id);
            }
            EventPayload::DecisionProposed(_)
            | EventPayload::DecisionRequested(_)
            | EventPayload::EvidenceRecorded(_)
            | EventPayload::HypothesisRecorded(_)
            | EventPayload::QuestionRecorded(_)
            | EventPayload::QuestionAsked(_)
            | EventPayload::RelationAdded(_)
            | EventPayload::RelationRemoved(_)
            | EventPayload::BlockerReported(_)
            | EventPayload::BlockerResolved(_)
            | EventPayload::NotificationSent(_)
            | EventPayload::NotificationAcknowledged(_)
            | EventPayload::SuggestionSurfaced(_)
            | EventPayload::IngestBatchReceived(_)
            | EventPayload::IngestBatchClassified(_)
            | EventPayload::DecisionScored(_)
            | EventPayload::DecisionAssessed(_)
            | EventPayload::DecisionMetadataDerived(_)
            | EventPayload::DecisionMoved(_)
            | EventPayload::DecisionRetitled(_)
            | EventPayload::DecisionReaffirmed(_)
            | EventPayload::ProjectRegistered(_)
            | EventPayload::ProjectLinked(_)
            | EventPayload::ProjectUnlinked(_)
            | EventPayload::ProjectAnchored(_)
            | EventPayload::ProjectUnanchored(_)
            | EventPayload::ProjectTopicDeclared(_) => {}
        }
    }
    Ok(reviewed)
}

#[derive(Debug, Default)]
struct ReviewLedgerContext {
    evidence: BTreeMap<String, String>,
    hypotheses: BTreeMap<String, String>,
}

impl ReviewLedgerContext {
    fn from_events(events: &[Event]) -> Result<Self> {
        let mut context = Self::default();
        for event in events {
            match validated_payload(event)? {
                EventPayload::EvidenceRecorded(payload) => {
                    context
                        .evidence
                        .insert(payload.evidence_id, payload.content);
                }
                EventPayload::HypothesisRecorded(payload) => {
                    context
                        .hypotheses
                        .insert(payload.hypothesis_id, payload.statement);
                }
                EventPayload::DecisionProposed(_)
                | EventPayload::DecisionRequested(_)
                | EventPayload::DecisionAccepted(_)
                | EventPayload::DecisionRejected(_)
                | EventPayload::DecisionSuperseded(_)
                | EventPayload::QuestionRecorded(_)
                | EventPayload::QuestionAsked(_)
                | EventPayload::RelationAdded(_)
                | EventPayload::RelationRemoved(_)
                | EventPayload::BlockerReported(_)
                | EventPayload::BlockerResolved(_)
                | EventPayload::NotificationSent(_)
                | EventPayload::NotificationAcknowledged(_)
                | EventPayload::SuggestionSurfaced(_)
                | EventPayload::IngestBatchReceived(_)
                | EventPayload::IngestBatchClassified(_)
                | EventPayload::DecisionScored(_)
                | EventPayload::DecisionAssessed(_)
                | EventPayload::DecisionMetadataDerived(_)
                | EventPayload::DecisionMoved(_)
                | EventPayload::DecisionRetitled(_)
                | EventPayload::DecisionReaffirmed(_)
                | EventPayload::ProjectRegistered(_)
                | EventPayload::ProjectLinked(_)
                | EventPayload::ProjectUnlinked(_)
                | EventPayload::ProjectAnchored(_)
                | EventPayload::ProjectUnanchored(_)
                | EventPayload::ProjectTopicDeclared(_) => {}
            }
        }
        Ok(context)
    }
}

fn render_review_item<W: IoWrite>(
    output: &mut W,
    index: usize,
    total: usize,
    item: &RecentDecisionEntry,
    context: &ReviewLedgerContext,
) -> Result<()> {
    writeln!(output, "\n[{index}/{total}] {}", item.decision_id).map_err(cli_io_error)?;
    writeln!(output, "Title: {}", item.title).map_err(cli_io_error)?;
    writeln!(output, "Status: {}", decision_status_label(item.status)).map_err(cli_io_error)?;
    writeln!(output, "Actors: {}", display_review_list(&item.actor_ids)).map_err(cli_io_error)?;
    writeln!(output, "Topics: {}", display_review_list(&item.topic_keys)).map_err(cli_io_error)?;
    writeln!(output, "Rationale: {}", item.rationale).map_err(cli_io_error)?;
    if let (Some(question), Some(quote)) = (&item.question, &item.quote) {
        writeln!(output, "Answers: {question}").map_err(cli_io_error)?;
        writeln!(output, "Quote: \"{quote}\"").map_err(cli_io_error)?;
    }
    writeln!(output, "Options:").map_err(cli_io_error)?;
    if item.options.is_empty() {
        writeln!(output, "  - <none>").map_err(cli_io_error)?;
    } else {
        for option in &item.options {
            let chosen = item.chosen_option_id.as_deref() == Some(option.option_id.as_str());
            write_review_option(output, option, chosen)?;
        }
    }
    writeln!(output, "Evidence:").map_err(cli_io_error)?;
    if item.evidence_ids.is_empty() {
        writeln!(output, "  - <none>").map_err(cli_io_error)?;
    } else {
        for evidence_id in &item.evidence_ids {
            match context.evidence.get(evidence_id) {
                Some(content) => {
                    writeln!(output, "  - {evidence_id}: {content}").map_err(cli_io_error)?
                }
                None => writeln!(output, "  - {evidence_id}").map_err(cli_io_error)?,
            }
        }
    }
    writeln!(output, "Hypotheses:").map_err(cli_io_error)?;
    if item.hypothesis_ids.is_empty() {
        writeln!(output, "  - <none>").map_err(cli_io_error)?;
    } else {
        for hypothesis_id in &item.hypothesis_ids {
            match context.hypotheses.get(hypothesis_id) {
                Some(statement) => {
                    writeln!(output, "  - {hypothesis_id}: {statement}").map_err(cli_io_error)?
                }
                None => writeln!(output, "  - {hypothesis_id}").map_err(cli_io_error)?,
            }
        }
    }
    Ok(())
}

/// One option line of the review prompt: the label a person reads, then what the choice and the
/// recorded form add. Written piece by piece, so no line is built as a `String` per option.
fn write_review_option<W: IoWrite>(
    output: &mut W,
    option: &OptionLabel,
    chosen: bool,
) -> Result<()> {
    write!(output, "  - {}", option.label).map_err(cli_io_error)?;
    match (&option.recorded_as, chosen) {
        (Some(recorded), true) => write!(output, " (chosen; recorded as: {recorded})"),
        (Some(recorded), false) => write!(output, " (recorded as: {recorded})"),
        (None, true) => write!(output, " (chosen)"),
        (None, false) => Ok(()),
    }
    .map_err(cli_io_error)?;
    writeln!(output).map_err(cli_io_error)
}

fn prompt_line<R: BufRead, W: IoWrite>(
    input: &mut R,
    output: &mut W,
    prompt: &str,
) -> Result<Option<String>> {
    write!(output, "{prompt}").map_err(cli_io_error)?;
    output.flush().map_err(cli_io_error)?;
    let mut line = String::new();
    let bytes_read = input.read_line(&mut line).map_err(cli_io_error)?;
    if bytes_read == 0 {
        return Ok(None);
    }
    Ok(Some(line.trim().to_owned()))
}

fn prompt_required_line<R: BufRead, W: IoWrite>(
    input: &mut R,
    output: &mut W,
    prompt: &str,
    empty_message: &str,
) -> Result<Option<String>> {
    loop {
        let Some(value) = prompt_line(input, output, prompt)? else {
            return Ok(None);
        };
        if let Some(value) = non_empty_owned(&value) {
            return Ok(Some(value));
        }
        writeln!(output, "{empty_message}").map_err(cli_io_error)?;
    }
}

fn split_review_list(value: &str) -> Vec<String> {
    value
        .split(',')
        .filter_map(non_empty_owned)
        .collect::<Vec<_>>()
}

fn non_empty_owned(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

fn display_review_list(values: &[String]) -> String {
    if values.is_empty() {
        "<none>".to_owned()
    } else {
        values.join(",")
    }
}

fn validated_payload(event: &Event) -> Result<EventPayload> {
    crate::events::validate(event).map_err(|error| {
        CliError::InvalidInput(format!(
            "ledger event {} failed validation during review: {error}",
            event.event_id.unwrap_or_default()
        ))
        .into()
    })
}

fn cli_io_error(error: io::Error) -> HivemindError {
    CliError::InvalidInput(format!("interactive review I/O failed: {error}")).into()
}

pub(crate) fn added_since_request(
    args: &QueryAddedSinceArgs,
) -> Result<DecisionsAddedSinceRequest> {
    let now = parse_utc_timestamp("--now", &args.now)?;
    let timezone = TimeZoneSpec::parse(&args.timezone)?;
    let since_timestamp = resolve_diff_bound(
        "--since",
        args.since.as_deref(),
        args.since_timestamp.as_deref(),
        now,
        timezone,
    )?;
    let until_timestamp = resolve_diff_bound(
        "--until",
        args.until.as_deref(),
        args.until_timestamp.as_deref(),
        now,
        timezone,
    )?;

    Ok(DecisionsAddedSinceRequest {
        since_offset: args.since_offset,
        since_timestamp,
        until_offset: args.until_offset,
        until_timestamp,
        filters: DecisionsAddedSinceFilterRequest {
            actor_ids: args.filters.actor_ids.clone(),
            sources: args.filters.sources.clone(),
            source_refs: args.filters.source_refs.clone(),
            import_run_ids: args.import_run_ids.clone(),
            topic_keys: args.filters.topic_keys.clone(),
            statuses: args
                .filters
                .statuses
                .iter()
                .copied()
                .map(QueryDecisionStatus::as_decision_status)
                .collect(),
        },
        limit: args.limit,
        cursor: args.cursor.clone(),
    })
}

/// `get_changed_decisions`'s window: `--since` (default 7 days back from `--now`) and `--until`,
/// each an RFC3339 timestamp, a `YYYY-MM-DD` date or a duration like `7d`.
fn changed_decisions_request(args: &QueryChangedDecisionsArgs) -> Result<ChangedDecisionsRequest> {
    let now = parse_utc_timestamp("--now", &args.now)?;
    let timezone = TimeZoneSpec::parse(&args.timezone)?;
    let since = resolve_diff_bound("--since", Some(args.since.as_str()), None, now, timezone)?;
    let until = resolve_diff_bound("--until", args.until.as_deref(), None, now, timezone)?;
    Ok(ChangedDecisionsRequest {
        since,
        until,
        limit: args.limit,
        cursor: args.cursor.clone(),
    })
}

pub(crate) fn recent_decisions_request(
    args: &QueryRecentDecisionsArgs,
) -> Result<RecentDecisionsRequest> {
    let now = parse_utc_timestamp("--now", &args.now)?;
    let timezone = TimeZoneSpec::parse(&args.timezone)?;
    let since_timestamp =
        resolve_diff_bound("--since", Some(args.since.as_str()), None, now, timezone)?
            .ok_or_else(|| CliError::InvalidInput("--since must not be empty".to_owned()))?;
    let until_timestamp =
        resolve_diff_bound("--until", args.until.as_deref(), None, now, timezone)?;

    Ok(RecentDecisionsRequest {
        since_timestamp,
        until_timestamp,
        filters: RecentDecisionFilterRequest {
            actor_patterns: args.actor_patterns.clone(),
            sources: args.sources.clone(),
            topic_keys: args.topic_keys.clone(),
            statuses: args
                .statuses
                .iter()
                .copied()
                .map(QueryDecisionStatus::as_decision_status)
                .collect(),
        },
        limit: args.limit,
        cursor: args.cursor.clone(),
    })
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum TimeZoneSpec {
    Utc,
}

impl TimeZoneSpec {
    pub(crate) fn parse(value: &str) -> Result<Self> {
        match value.trim() {
            "" | "UTC" | "utc" | "Etc/UTC" => Ok(Self::Utc),
            other => Err(CliError::InvalidInput(format!(
                "--timezone {other} is not supported in slice 1; only UTC is accepted"
            ))
            .into()),
        }
    }
}

pub(crate) fn resolve_diff_bound(
    flag: &'static str,
    raw: Option<&str>,
    explicit_ts: Option<&str>,
    now: Option<DateTime<Utc>>,
    timezone: TimeZoneSpec,
) -> Result<Option<DateTime<Utc>>> {
    if let Some(ts) = explicit_ts {
        return parse_utc_timestamp(flag, &Some(ts.to_owned()));
    }
    let Some(value) = raw.map(str::trim) else {
        return Ok(None);
    };
    if value.is_empty() {
        return Ok(None);
    }
    if let Ok(parsed) = DateTime::parse_from_rfc3339(value) {
        return Ok(Some(parsed.with_timezone(&Utc)));
    }
    if let Some(parsed) = parse_utc_date(value) {
        return Ok(Some(parsed));
    }
    let now = now.unwrap_or_else(Utc::now);
    if let Some(resolved) = resolve_relative_duration(value, now) {
        return Ok(Some(resolved));
    }
    let resolved = resolve_relative_phrase(value, now, timezone).ok_or_else(|| {
        CliError::InvalidInput(format!(
            "{flag} must be an RFC3339 timestamp, YYYY-MM-DD date, duration like 7d/24h, or supported phrase (got: {value})"
        ))
    })?;
    Ok(Some(resolved))
}

fn parse_utc_date(value: &str) -> Option<DateTime<Utc>> {
    use chrono::{NaiveDate, NaiveTime, TimeZone};
    NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .ok()
        .map(|date| Utc.from_utc_datetime(&date.and_time(NaiveTime::MIN)))
}

/// Parses `--check-by` with the one check-date parser (`crate::util::parse_check_by`).
fn parse_check_by_date(flag: &'static str, value: &str) -> Result<DateTime<Utc>> {
    crate::util::parse_check_by(value).ok_or_else(|| {
        CliError::InvalidInput(format!(
            "{flag} must be an RFC3339 timestamp or a YYYY-MM-DD date (got: {value})"
        ))
        .into()
    })
}

fn resolve_relative_duration(value: &str, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let (amount, unit) = value.split_at(value.len().checked_sub(1)?);
    let amount = amount.parse::<i64>().ok()?;
    if amount < 0 {
        return None;
    }
    let duration = match unit.to_ascii_lowercase().as_str() {
        "s" => chrono::Duration::seconds(amount),
        "m" => chrono::Duration::minutes(amount),
        "h" => chrono::Duration::hours(amount),
        "d" => chrono::Duration::days(amount),
        "w" => chrono::Duration::weeks(amount),
        _ => return None,
    };
    now.checked_sub_signed(duration)
}

fn resolve_relative_phrase(
    phrase: &str,
    now: DateTime<Utc>,
    timezone: TimeZoneSpec,
) -> Option<DateTime<Utc>> {
    let normalized = phrase.trim().to_ascii_lowercase();
    let TimeZoneSpec::Utc = timezone;
    match normalized.as_str() {
        "now" => Some(now),
        "last week" | "last_week" | "last-week" => Some(start_of_previous_iso_week_utc(now)),
        "this week" | "this_week" | "this-week" => Some(start_of_current_iso_week_utc(now)),
        "yesterday" => Some(start_of_day_utc(now) - chrono::Duration::days(1)),
        "today" => Some(start_of_day_utc(now)),
        _ => None,
    }
}

fn start_of_day_utc(now: DateTime<Utc>) -> DateTime<Utc> {
    use chrono::NaiveTime;
    use chrono::TimeZone;
    Utc.from_utc_datetime(&now.date_naive().and_time(NaiveTime::MIN))
}

fn start_of_current_iso_week_utc(now: DateTime<Utc>) -> DateTime<Utc> {
    use chrono::{Datelike, NaiveTime, TimeZone};
    let date = now.date_naive();
    let days_from_monday = i64::from(date.weekday().num_days_from_monday());
    let monday = date - chrono::Duration::days(days_from_monday);
    Utc.from_utc_datetime(&monday.and_time(NaiveTime::MIN))
}

fn start_of_previous_iso_week_utc(now: DateTime<Utc>) -> DateTime<Utc> {
    start_of_current_iso_week_utc(now) - chrono::Duration::days(7)
}

fn recent_activity_request(args: &QueryRecentActivityArgs) -> Result<RecentActivityRequest> {
    Ok(RecentActivityRequest {
        filters: history_filter_request(&args.filters),
        limit: args.limit,
        cursor: args.cursor.clone(),
    })
}

fn changed_since_request(args: &QueryChangedSinceArgs) -> Result<ChangedSinceRequest> {
    Ok(ChangedSinceRequest {
        since_offset: args.since_offset,
        since_timestamp: parse_utc_timestamp("--since-ts", &args.since_timestamp)?,
        until_offset: args.until_offset,
        until_timestamp: parse_utc_timestamp("--until-ts", &args.until_timestamp)?,
        filters: history_filter_request(&args.filters),
        limit: args.limit,
        cursor: args.cursor.clone(),
    })
}

fn export_read_only_summary_request(
    args: &QueryExportReadOnlySummaryArgs,
) -> Result<ReadOnlyExportRequest> {
    let generated_at =
        parse_utc_timestamp("--generated-at", &args.generated_at)?.unwrap_or_else(Utc::now);
    let filters = history_filter_request(&args.filters);
    let query = match args.query {
        QueryExportKind::RecentActivity => {
            ReadOnlyExportQuery::RecentActivity(RecentActivityRequest {
                filters,
                limit: args.limit,
                cursor: args.cursor.clone(),
            })
        }
        QueryExportKind::DecisionsChangedSince => {
            ReadOnlyExportQuery::DecisionsChangedSince(ChangedSinceRequest {
                since_offset: args.since_offset,
                since_timestamp: parse_utc_timestamp("--since-ts", &args.since_timestamp)?,
                until_offset: args.until_offset,
                until_timestamp: parse_utc_timestamp("--until-ts", &args.until_timestamp)?,
                filters,
                limit: args.limit,
                cursor: args.cursor.clone(),
            })
        }
    };

    Ok(ReadOnlyExportRequest {
        query,
        format: args.format.as_query_format(),
        generated_at,
    })
}

fn history_filter_request(args: &QueryHistoryFilterArgs) -> HistoryFilterRequest {
    HistoryFilterRequest {
        actor_ids: args.actor_ids.clone(),
        sources: args.sources.clone(),
        source_refs: args.source_refs.clone(),
        topic_keys: args.topic_keys.clone(),
        statuses: args
            .statuses
            .iter()
            .copied()
            .map(QueryDecisionStatus::as_decision_status)
            .collect(),
    }
}

fn parse_utc_timestamp(
    field: &'static str,
    value: &Option<String>,
) -> Result<Option<DateTime<Utc>>> {
    match value.as_deref() {
        None => Ok(None),
        Some(value) => DateTime::parse_from_rfc3339(value)
            .map(|timestamp| Some(timestamp.with_timezone(&Utc)))
            .map_err(|error| {
                CliError::InvalidInput(format!("{field} must be an RFC 3339 timestamp: {error}"))
                    .into()
            }),
    }
}

/// The findings a `scan_decision_quality` or `get_suggestions` command asks for.
fn scan_request(args: &QueryScanDecisionQualityArgs) -> Result<ScanRequest> {
    Ok(ScanRequest {
        kinds: parse_kinds(&args.kinds).map_err(CliError::InvalidInput)?,
        limit: args.limit,
        cursor: args.cursor.clone(),
        evidence_window_days: args.evidence_window_days,
    })
}

/// The page of the importance ranking a `rank_decisions_by_importance` command asks for.
fn importance_request(args: &QueryRankDecisionsByImportanceArgs) -> importance::ImportanceRequest {
    importance::ImportanceRequest {
        include_not_in_force: args.include_not_in_force,
        limit: args.limit,
        cursor: args.cursor.clone(),
    }
}

fn run_query_with_graph(
    context: &QueryContext,
    ledger: &AnyLedger,
    graph: &impl GraphView,
    hivemind_dir: &Path,
    query: &QueryArgs,
) -> Result<String> {
    let output = match &query.command {
        QueryCommand::GetDecision(args) => {
            let response = get_decision(graph, &args.decision_id)?;
            format_query_response(query.summary, &response, render_decision_summary, None)?
        }
        QueryCommand::GetRelevantDecisions(args) => {
            let response = get_relevant_decisions(
                graph,
                &args.topic,
                args.status.map(QueryDecisionStatus::as_decision_status),
            )?;
            format_query_response(
                query.summary,
                &response,
                |decisions| render_decision_list_summary(decisions),
                None,
            )?
        }
        QueryCommand::GetSupersessionChain(args) => {
            let (target, close_match) = resolve_fluent_target_for_reading(
                hivemind_dir,
                query.summary,
                graph,
                args.decision_id.as_deref(),
                args.description.as_deref(),
                args.pick,
                args.topic.as_deref(),
            )?;
            let decision_id = match target {
                FluentResolution::Id(decision_id) => decision_id,
                FluentResolution::Output(output) => return Ok(output),
            };
            let response = get_supersession_chain(graph, &decision_id)?;
            format_close_matched_response(
                query.summary,
                &response,
                close_match.as_ref(),
                render_supersession_summary,
            )?
        }
        QueryCommand::GetDecisionNeighborhood(args) => {
            let (target, close_match) = resolve_fluent_target_for_reading(
                hivemind_dir,
                query.summary,
                graph,
                args.decision_id.as_deref(),
                args.description.as_deref(),
                args.pick,
                args.topic.as_deref(),
            )?;
            let decision_id = match target {
                FluentResolution::Id(decision_id) => decision_id,
                FluentResolution::Output(output) => return Ok(output),
            };
            if args.compact {
                let response = get_compact_view(graph, &decision_id)?;
                format_close_matched_response(
                    query.summary,
                    &response,
                    close_match.as_ref(),
                    render_compact_view_summary,
                )?
            } else {
                if args.depth != 1 {
                    return Err(CliError::InvalidInput(format!(
                        "--depth {} is not supported yet; slice-1 only supports depth=1 with hypothesis SUPPORTS/REFUTES auto-expanded",
                        args.depth
                    ))
                    .into());
                }
                let request = if args.relations.is_empty() {
                    NeighborhoodRequest::all()
                } else {
                    NeighborhoodRequest::with_relations(
                        args.relations
                            .iter()
                            .copied()
                            .map(QueryRelationKind::as_graph_relation),
                    )
                };
                let mut response = get_decision_neighborhood(graph, &decision_id, &request)?;
                // The graph holds no per-edge times: the ledger supplies the dated story.
                let scoped_ledger = TenantScopedLedger::new(ledger, context.tenant_id.clone());
                response.data.timeline =
                    get_decision_timeline(graph, &scoped_ledger, &decision_id)?.data;
                // The answer to "why" carries how well the record supports the decision.
                let response = quality_profile::why_with_quality(graph, response)?;
                format_close_matched_response(
                    query.summary,
                    &response,
                    close_match.as_ref(),
                    render_neighborhood_summary,
                )?
            }
        }
        QueryCommand::GetCompactView(args) => {
            let (target, close_match) = resolve_fluent_target_for_reading(
                hivemind_dir,
                query.summary,
                graph,
                args.decision_id.as_deref(),
                args.description.as_deref(),
                args.pick,
                args.topic.as_deref(),
            )?;
            let decision_id = match target {
                FluentResolution::Id(decision_id) => decision_id,
                FluentResolution::Output(output) => return Ok(output),
            };
            let response = get_compact_view(graph, &decision_id)?;
            format_close_matched_response(
                query.summary,
                &response,
                close_match.as_ref(),
                render_compact_view_summary,
            )?
        }
        QueryCommand::GetDecisionOutcome(args) => {
            let (target, close_match) = resolve_fluent_target_for_reading(
                hivemind_dir,
                query.summary,
                graph,
                args.decision_id.as_deref(),
                args.description.as_deref(),
                args.pick,
                args.topic.as_deref(),
            )?;
            let decision_id = match target {
                FluentResolution::Id(decision_id) => decision_id,
                FluentResolution::Output(output) => return Ok(output),
            };
            let response = get_decision_brief(graph, &decision_id)?;
            // The answer carries how well the record supports the decision.
            let response = quality_profile::verify_with_quality(graph, response)?;
            format_close_matched_response(
                query.summary,
                &response,
                close_match.as_ref(),
                render_decision_brief_summary,
            )?
        }
        QueryCommand::Search(args) => {
            let request = search_decision_request(args)?;
            let response = search_decisions_any(context, ledger, graph, &request)?;
            format_query_response(
                query.summary,
                &response,
                render_search_summary,
                response.data.next_cursor.as_deref(),
            )?
        }
        QueryCommand::SearchDecisions(args) => {
            let request = search_decision_request(args)?;
            let response = search_decisions_any(context, ledger, graph, &request)?;
            format_query_response(
                query.summary,
                &response,
                render_search_summary,
                response.data.next_cursor.as_deref(),
            )?
        }
        QueryCommand::Recall(args) => {
            let limit = args.limit.clamp(1, RECALL_MAX_LIMIT);
            let request = RecallRequest {
                q: args.query.clone(),
                topic_keys: args.topic_keys.clone(),
                statuses: args
                    .statuses
                    .iter()
                    .copied()
                    .map(QueryDecisionStatus::as_decision_status)
                    .collect(),
                actor_ids: args.actor_ids.clone(),
                sources: args.sources.clone(),
                since: parse_query_datetime(args.since.as_deref(), "--since")?,
                until: parse_query_datetime(args.until.as_deref(), "--until")?,
                limit,
                cursor: args.cursor.clone(),
                project: args.project.clone(),
            };
            let response = recall_decisions(context, ledger, graph, &request)?;
            format_query_response(
                query.summary,
                &response,
                render_recall_summary,
                response.data.ranked.next_cursor.as_deref(),
            )?
        }
        QueryCommand::GetActiveDecisionBlockers(args) => {
            let request = ActiveDecisionBlockersRequest {
                filters: DecisionBlockerFilters {
                    decision_ids: args.decision_ids.clone(),
                    topic_keys: args.topic_keys.clone(),
                    required_owner_ids: args.required_owner_ids.clone(),
                    blocked_actor_ids: args.blocked_actor_ids.clone(),
                    priorities: args
                        .priorities
                        .iter()
                        .copied()
                        .map(QueryBlockerPriority::as_blocker_priority)
                        .collect(),
                    now: parse_query_datetime(args.now.as_deref(), "--now")?,
                    stale_after_seconds: args.stale_after_seconds,
                },
                limit: args.limit,
                cursor: args.cursor.clone(),
            };
            let response = get_active_decision_blockers(graph, &request)?;
            format_query_response(
                query.summary,
                &response,
                render_active_blockers_summary,
                response.data.next_cursor.as_deref(),
            )?
        }
        QueryCommand::GetWaitingRequests(args) => {
            let request = WaitingRequestsRequest {
                limit: args.limit,
                cursor: args.cursor.clone(),
            };
            let response = get_waiting_requests(graph, &request)?;
            format_query_response(
                query.summary,
                &response,
                render_waiting_requests_summary,
                response.data.next_cursor.as_deref(),
            )?
        }
        QueryCommand::GetContestedDecisions(args) => {
            let request = ContestedDecisionsRequest {
                limit: args.limit,
                cursor: args.cursor.clone(),
            };
            let response = get_contested_decisions(graph, &request)?;
            format_query_response(
                query.summary,
                &response,
                render_contested_decisions_summary,
                response.data.next_cursor.as_deref(),
            )?
        }
        QueryCommand::GetOpenWhyDecisions(args) => {
            let request = OpenWhyRequest {
                limit: args.limit,
                cursor: args.cursor.clone(),
            };
            let response = get_open_why_decisions(graph, &request)?;
            format_query_response(
                query.summary,
                &response,
                render_open_why_summary,
                response.data.next_cursor.as_deref(),
            )?
        }
        QueryCommand::GetChangedDecisions(args) => {
            let scoped_ledger = TenantScopedLedger::new(ledger, context.tenant_id.clone());
            let response =
                get_changed_decisions(graph, &scoped_ledger, &changed_decisions_request(args)?)?;
            format_query_response(
                query.summary,
                &response,
                render_changed_decisions_summary,
                response.data.next_cursor.as_deref(),
            )?
        }
        QueryCommand::GetBlockerNotificationCandidates(args) => {
            let request = BlockerNotificationCandidatesRequest {
                now: parse_required_query_datetime(&args.now, "--now")?,
                policy_version: args.policy_version.clone(),
                limit: args.limit,
                cursor: args.cursor.clone(),
            };
            let response = get_blocker_notification_candidates(graph, &request)?;
            format_query_response(
                query.summary,
                &response,
                render_blocker_notifications_summary,
                response.data.next_cursor.as_deref(),
            )?
        }
        QueryCommand::ScoreDecision(args) => {
            let response = quality_profile::score_decision(graph, &args.decision_id)?;
            format_query_response(query.summary, &response, render_score_report_summary, None)?
        }
        QueryCommand::ScanDecisionQuality(args) => {
            let request = scan_request(args)?;
            let response = quality_profile::scan_decision_quality(graph, &request)?;
            format_query_response(
                query.summary,
                &response,
                render_scan_report_summary,
                response.data.next_cursor.as_deref(),
            )?
        }
        QueryCommand::GetSuggestions(args) => {
            let request = SuggestionsRequest {
                scan: scan_request(&args.scan)?,
                exclude_acknowledged: args.exclude_acknowledged,
            };
            let response = quality_profile::get_suggestions(graph, &request)?;
            format_query_response(
                query.summary,
                &response,
                render_scan_report_summary,
                response.data.next_cursor.as_deref(),
            )?
        }
        QueryCommand::RankDecisionsByImportance(args) => {
            let request = importance_request(args);
            let response = importance::rank_decisions_by_importance(graph, &request)?;
            format_query_response(
                query.summary,
                &response,
                render_importance_summary,
                response.data.next_cursor.as_deref(),
            )?
        }
        QueryCommand::ScanMisfiledDecisions(args) => {
            let request = MisfiledScanRequest {
                foreign_topic_keys: args.foreign_topic_keys.clone(),
                project: args.project.clone(),
                move_to: args.move_to.clone(),
                limit: args.limit,
                cursor: args.cursor.clone(),
            };
            // The scan reads the graph, which has no registry: a handle is checked here, so a
            // typo is a refusal and never "nothing misfiled".
            let scoped_ledger = TenantScopedLedger::new(ledger, context.tenant_id.clone());
            for handle in request.project.iter().chain(request.move_to.iter()) {
                require_registered_project(&scoped_ledger, handle)?;
            }
            let response = scan_misfiled_decisions(graph, &request)?;
            let skip: usize = args
                .cursor
                .as_deref()
                .and_then(|c| c.parse().ok())
                .unwrap_or(0);
            let next_cursor = if response.truncated {
                misfiled_next_cursor(skip, response.result_count)
            } else {
                None
            };
            format_query_response(
                query.summary,
                &response,
                |d: &Vec<_>| render_misfiled_scan_summary(d),
                next_cursor.as_deref(),
            )?
        }
        QueryCommand::GetSituationalDecisions(args) => {
            let request = situational_request(args)?;
            let response = get_situational_decisions(context, graph, ledger, &request)?;
            format_query_response(
                query.summary,
                &response,
                render_situational_summary,
                response.data.next_cursor.as_deref(),
            )?
        }
        QueryCommand::RecentDecisions(args) => {
            let scoped_ledger = TenantScopedLedger::new(ledger, context.tenant_id.clone());
            let response =
                get_recent_decisions(&scoped_ledger, graph, &recent_decisions_request(args)?)?;
            format_query_response(
                query.summary,
                &response,
                render_recent_decisions_summary,
                response.data.next_cursor.as_deref(),
            )?
        }
        QueryCommand::GetDecisionsAddedSince(args) => {
            let scoped_ledger = TenantScopedLedger::new(ledger, context.tenant_id.clone());
            let response =
                get_decisions_added_since(&scoped_ledger, graph, &added_since_request(args)?)?;
            format_query_response(
                query.summary,
                &response,
                render_added_since_summary,
                response.data.next_cursor.as_deref(),
            )?
        }
        QueryCommand::GetRecentActivity(_)
        | QueryCommand::GetDecisionsChangedSince(_)
        | QueryCommand::ExportReadOnlySummary(_) => {
            return Err(
                CliError::InvalidInput("query requires ledger-backed execution".to_owned()).into(),
            )
        }
    };

    Ok(output)
}

fn parse_query_datetime(value: Option<&str>, flag: &str) -> Result<Option<DateTime<Utc>>> {
    value
        .map(|value| parse_required_query_datetime(value, flag))
        .transpose()
}

fn search_decision_request(args: &QuerySearchDecisionsArgs) -> Result<SearchDecisionRequest> {
    Ok(SearchDecisionRequest {
        query: args.query.clone(),
        topic_keys: args.topic_keys.clone(),
        statuses: args
            .statuses
            .iter()
            .copied()
            .map(QueryDecisionStatus::as_decision_status)
            .collect(),
        actor_ids: args.actor_ids.clone(),
        sources: args.sources.clone(),
        since: parse_query_datetime(args.since.as_deref(), "--since")?,
        until: parse_query_datetime(args.until.as_deref(), "--until")?,
        limit: args.limit,
        cursor: args.cursor.clone(),
        project: None,
    })
}

/// Build a `SituationalRequest` from CLI flags, resolving cwd/diff/branch into plain
/// path-like strings here (the only place in this feature that shells out to `git` —
/// `queries::situational` itself never touches the process/filesystem, per the
/// three-layer separation in AGENTS.md).
fn situational_request(args: &QuerySituationalArgs) -> Result<SituationalRequest> {
    let paths = resolve_situational_paths(args)?;
    let (since_offset, since_timestamp) = resolve_situational_since(args)?;
    Ok(SituationalRequest {
        paths,
        since_offset,
        since_timestamp,
        limit: args.limit,
        cursor: args.cursor.clone(),
        project: args.project.clone(),
    })
}

fn resolve_situational_paths(args: &QuerySituationalArgs) -> Result<Vec<String>> {
    let mut paths: Vec<String> = args
        .paths
        .iter()
        .map(|path| path.trim().to_owned())
        .filter(|path| !path.is_empty())
        .collect();

    if args.diff {
        let mut buf = String::new();
        io::stdin().read_to_string(&mut buf).map_err(|error| {
            CliError::InvalidInput(format!("failed to read diff from stdin: {error}"))
        })?;
        paths.extend(diff_touched_paths(&buf));
    }

    if args.branch {
        paths.push(git_current_branch()?);
    }

    if args.cwd {
        let cwd = std::env::current_dir()
            .map_err(|error| CliError::InvalidInput(format!("failed to read cwd: {error}")))?;
        paths.push(cwd.to_string_lossy().into_owned());
    }

    if paths.is_empty() && !args.diff && !args.branch && !args.cwd {
        paths = git_default_diff_paths()?;
    }

    if paths.is_empty() {
        return Err(CliError::InvalidInput(
            "situational query found no paths to work from (no --paths given, no local git diff/staged changes); pass --paths or --diff explicitly".to_owned(),
        )
        .into());
    }

    Ok(paths)
}

/// Extract touched file paths from a unified diff's `+++ b/...` / `--- a/...` headers.
fn diff_touched_paths(diff_text: &str) -> Vec<String> {
    let mut paths = Vec::new();
    for line in diff_text.lines() {
        for prefix in ["+++ b/", "--- a/"] {
            if let Some(rest) = line.strip_prefix(prefix) {
                let rest = rest.trim();
                if !rest.is_empty() {
                    paths.push(rest.to_owned()); // ubs:ignore: allocation necessary — rest borrows from diff_text; the function returns Vec<String> so callers can own the paths independently of the diff buffer's lifetime
                }
            }
        }
    }
    paths.sort();
    paths.dedup();
    paths
}

fn git_current_branch() -> Result<String> {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .output()
        .map_err(|error| CliError::InvalidInput(format!("failed to run git: {error}")))?;
    if !output.status.success() {
        return Err(CliError::InvalidInput(
            "--branch requires a git repository (git rev-parse failed); pass --paths/--diff explicitly instead".to_owned(),
        )
        .into());
    }
    let branch = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if branch.is_empty() {
        return Err(CliError::InvalidInput(
            "git rev-parse returned an empty branch name".to_owned(),
        )
        .into());
    }
    Ok(branch)
}

/// Default "no question needed" path set: the current git diff plus the staged set.
/// Returns a clear, non-panicking error (not a silent empty result) when this isn't a
/// git repository, so the caller knows to pass `--paths`/`--diff` explicitly.
fn git_default_diff_paths() -> Result<Vec<String>> {
    let Some(mut paths) = run_git_diff_name_only(&[])? else {
        return Err(CliError::InvalidInput(
            "situational query defaults to the current git diff/staged set, but this is not a git repository (or git is not on PATH); pass --paths/--diff explicitly".to_owned(),
        )
        .into());
    };
    if let Some(staged) = run_git_diff_name_only(&["--cached"])? {
        paths.extend(staged);
    }
    paths.sort();
    paths.dedup();
    Ok(paths)
}

/// Runs `git diff --name-only [extra_args]`. `Ok(None)` means git itself could not be
/// invoked or failed (not a git repo, no `git` on PATH) — distinct from `Ok(Some(vec![]))`,
/// a clean repo with nothing to report.
fn run_git_diff_name_only(extra_args: &[&str]) -> Result<Option<Vec<String>>> {
    let mut command = std::process::Command::new("git");
    command.arg("diff").arg("--name-only").args(extra_args);
    let output = match command.output() {
        Ok(output) if output.status.success() => output,
        _ => return Ok(None),
    };
    let paths = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|line| line.trim().to_owned())
        .filter(|line| !line.is_empty())
        .collect();
    Ok(Some(paths))
}

fn resolve_situational_since(
    args: &QuerySituationalArgs,
) -> Result<(Option<u64>, Option<DateTime<Utc>>)> {
    if args.since_branch_point {
        if args.since_offset.is_some() || args.since_timestamp.is_some() {
            return Err(CliError::InvalidInput(
                "--since-branch-point is mutually exclusive with --since-offset/--since-ts"
                    .to_owned(),
            )
            .into());
        }
        return Ok((None, Some(git_branch_point_timestamp(&args.base)?)));
    }
    Ok((
        args.since_offset,
        parse_utc_timestamp("--since-ts", &args.since_timestamp)?,
    ))
}

/// Resolves `--since-branch-point` to the timestamp of the commit where the current
/// branch diverged from `base`, so "what changed since I last worked here" is one call
/// (`get_decisions_changed_since` tests each event's own `ts` against it, so a back-dated
/// event cannot move the boundary).
fn git_branch_point_timestamp(base: &str) -> Result<DateTime<Utc>> {
    let merge_base = std::process::Command::new("git")
        .args(["merge-base", "HEAD", base])
        .output()
        .map_err(|error| {
            CliError::InvalidInput(format!("failed to run git merge-base: {error}"))
        })?;
    if !merge_base.status.success() {
        return Err(CliError::InvalidInput(format!(
            "--since-branch-point requires a git repository and a resolvable base ref '{base}'"
        ))
        .into());
    }
    let merge_base_sha = String::from_utf8_lossy(&merge_base.stdout)
        .trim()
        .to_owned();

    let log = std::process::Command::new("git")
        .args(["log", "-1", "--format=%aI", &merge_base_sha])
        .output()
        .map_err(|error| CliError::InvalidInput(format!("failed to run git log: {error}")))?;
    if !log.status.success() {
        return Err(CliError::InvalidInput(format!(
            "git log failed to resolve the merge-base commit {merge_base_sha}"
        ))
        .into());
    }
    let timestamp = String::from_utf8_lossy(&log.stdout).trim().to_owned();
    parse_utc_timestamp("--since-branch-point", &Some(timestamp))?.ok_or_else(|| {
        CliError::InvalidInput("git log returned an empty merge-base commit timestamp".to_owned())
            .into()
    })
}

fn parse_required_query_datetime(value: &str, flag: &str) -> Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .map(|value| value.with_timezone(&Utc))
        .map_err(|error| {
            CliError::InvalidInput(format!("{flag} must be an RFC3339 timestamp: {error}")).into()
        })
}

fn run_dump(cli: &Cli, dump: &DumpArgs) -> Result<String> {
    let tenant_id = cli_tenant(cli)?;
    let ledger = open_ledger(cli)?;

    match selected_graph_backend(cli)? {
        GraphBackend::Memory => {
            let graph = MemoryGraph::default();
            rebuild_graph_for_tenant(&ledger, &tenant_id, &graph)?;
            run_dump_with_graph(&graph, dump)
        }
        GraphBackend::Kuzu => run_dump_with_kuzu(&tenant_id, &ledger, &cli.hivemind_dir, dump),
    }
}

#[cfg(feature = "tui")]
fn run_tui(cli: &Cli, args: &TuiArgs) -> Result<String> {
    if cli.json {
        return Err(CliError::InvalidInput(
            "--json is not supported for the interactive tui command".to_owned(),
        )
        .into());
    }

    let tenant_id = cli_tenant(cli)?;
    let ledger = open_ledger(cli)?;
    let config = crate::tui::TuiConfig {
        query: args.query.clone(),
        topic_keys: args.topic_keys.clone(),
        statuses: args
            .statuses
            .iter()
            .copied()
            .map(QueryDecisionStatus::as_decision_status)
            .collect(),
        actor_ids: args.actor_ids.clone(),
        sources: args.sources.clone(),
        limit: args.limit,
        dot_output: args.dot_output.clone(),
    };

    match selected_graph_backend(cli)? {
        GraphBackend::Memory => {
            let graph = MemoryGraph::default();
            rebuild_graph_for_tenant(&ledger, &tenant_id, &graph)?;
            crate::tui::run(&graph, config)?;
        }
        GraphBackend::Kuzu => run_tui_with_kuzu(&tenant_id, &ledger, &cli.hivemind_dir, config)?,
    }

    Ok("tui exited".to_owned())
}

#[cfg(not(feature = "tui"))]
fn run_tui(_cli: &Cli, _args: &TuiArgs) -> Result<String> {
    Err(
        CliError::InvalidInput("tui command requires building with --features tui".to_owned())
            .into(),
    )
}

#[cfg(all(feature = "tui", feature = "graph-kuzu"))]
fn run_tui_with_kuzu(
    tenant_id: &TenantId,
    ledger: &impl EventLedger,
    hivemind_dir: &std::path::Path,
    config: crate::tui::TuiConfig,
) -> Result<()> {
    let graph = crate::projector::kuzu::KuzuGraph::open(hivemind_dir)?;
    rebuild_graph_for_tenant(ledger, tenant_id, &graph)?;
    crate::tui::run(&graph, config)
}

#[cfg(all(feature = "tui", not(feature = "graph-kuzu")))]
fn run_tui_with_kuzu(
    _tenant_id: &TenantId,
    _ledger: &impl EventLedger,
    _hivemind_dir: &std::path::Path,
    _config: crate::tui::TuiConfig,
) -> Result<()> {
    Err(CliError::InvalidInput(
        "graph backend 'kuzu' requires building with --features graph-kuzu".to_owned(),
    )
    .into())
}

fn run_dump_with_graph(graph: &impl GraphView, dump: &DumpArgs) -> Result<String> {
    match dump.format {
        DumpFormat::Dot => render_dot(graph),
    }
}

#[cfg(feature = "graph-kuzu")]
fn run_query_with_kuzu(
    context: &QueryContext,
    ledger: &AnyLedger,
    hivemind_dir: &std::path::Path,
    query: &QueryArgs,
) -> Result<String> {
    let graph = crate::projector::kuzu::KuzuGraph::open(hivemind_dir)?;
    rebuild_graph_for_tenant(ledger, &context.tenant_id, &graph)?;
    run_query_with_graph(context, ledger, &graph, hivemind_dir, query)
}

#[cfg(not(feature = "graph-kuzu"))]
fn run_query_with_kuzu(
    _context: &QueryContext,
    _ledger: &AnyLedger,
    _hivemind_dir: &std::path::Path,
    _query: &QueryArgs,
) -> Result<String> {
    Err(CliError::InvalidInput(
        "graph backend 'kuzu' requires building with --features graph-kuzu".to_owned(),
    )
    .into())
}

#[cfg(feature = "graph-kuzu")]
fn run_dump_with_kuzu(
    tenant_id: &TenantId,
    ledger: &impl EventLedger,
    hivemind_dir: &std::path::Path,
    dump: &DumpArgs,
) -> Result<String> {
    let graph = crate::projector::kuzu::KuzuGraph::open(hivemind_dir)?;
    rebuild_graph_for_tenant(ledger, tenant_id, &graph)?;
    run_dump_with_graph(&graph, dump)
}

#[cfg(not(feature = "graph-kuzu"))]
fn run_dump_with_kuzu(
    _tenant_id: &TenantId,
    _ledger: &impl EventLedger,
    _hivemind_dir: &std::path::Path,
    _dump: &DumpArgs,
) -> Result<String> {
    Err(CliError::InvalidInput(
        "graph backend 'kuzu' requires building with --features graph-kuzu".to_owned(),
    )
    .into())
}

fn decision_status_after_write(
    ledger: &impl EventLedger,
    tenant_id: &TenantId,
    decision_id: &str,
) -> Result<DecisionStatus> {
    let graph = MemoryGraph::default();
    rebuild_graph_for_tenant(ledger, tenant_id, &graph)?;
    derive_decision_status(&graph, decision_id)
}

fn parse_actor_mappings(values: &[String]) -> Result<BTreeMap<String, String>> {
    let mut mappings = BTreeMap::new();
    for value in values {
        let (slack_user, actor_id) = value.split_once('=').ok_or_else(|| {
            CliError::InvalidInput(
                "--actor-map must use SlackUser=HiveMindActorId format".to_owned(),
            )
        })?;
        let slack_user = trimmed_required("--actor-map Slack user", slack_user)?;
        let actor_id = trimmed_required("--actor-map actor id", actor_id)?;
        mappings.insert(slack_user.to_owned(), actor_id.to_owned());
    }
    Ok(mappings)
}

fn validate_global_flags(cli: &Cli) -> Result<()> {
    if cli.actor.trim().is_empty() {
        return Err(CliError::InvalidInput("--actor must not be empty".to_owned()).into());
    }
    cli_tenant(cli)?;

    Ok(())
}

pub(crate) fn cli_tenant(cli: &Cli) -> Result<TenantId> {
    TenantId::new(cli.tenant.trim().to_owned())
        .map_err(|error| CliError::InvalidInput(format!("--tenant is invalid: {error}")).into())
}

/// Opens the ledger for one CLI invocation: SQLite under `--hivemind-dir` (default)
/// or Postgres when `--database-url`/`HIVEMIND_DATABASE_URL` is set. Every command
/// that reads or writes the ledger goes through this — a CLI that captures to
/// SQLite but reads Postgres (or vice versa) is worse than either alone.
fn open_ledger(cli: &Cli) -> Result<AnyLedger> {
    AnyLedger::open(&LedgerConfig::from_cli(cli), &cli_tenant(cli)?)
}

fn cli_command_context(cli: &Cli, provenance: EventProvenance) -> Result<CommandContext> {
    Ok(CommandContext::new(cli_tenant(cli)?, provenance))
}

fn cli_query_context(cli: &Cli) -> Result<QueryContext> {
    Ok(QueryContext::new(cli_tenant(cli)?))
}

fn selected_graph_backend(cli: &Cli) -> Result<GraphBackend> {
    if let Some(backend) = cli.graph_backend {
        return Ok(backend);
    }

    match std::env::var("HIVEMIND_GRAPH_BACKEND") {
        Ok(value) => parse_graph_backend(&value),
        Err(std::env::VarError::NotPresent) => Ok(GraphBackend::Memory),
        Err(error) => Err(CliError::InvalidInput(format!(
            "HIVEMIND_GRAPH_BACKEND is not valid unicode: {error}"
        ))
        .into()),
    }
}

pub(crate) fn parse_graph_backend(value: &str) -> Result<GraphBackend> {
    match value.trim().to_ascii_lowercase().as_str() {
        "" | "memory" | "in-memory" | "in_memory" => Ok(GraphBackend::Memory),
        "kuzu" | "graph-kuzu" | "graph_kuzu" => Ok(GraphBackend::Kuzu),
        other => Err(CliError::InvalidInput(format!(
            "unknown graph backend '{other}'; expected 'memory' or 'kuzu'"
        ))
        .into()),
    }
}

/// Public DOT renderer for callers outside the CLI module (e.g. the MCP
/// server). Delegates to the same internal implementation `hivemind dump`
/// uses so output stays identical across transports.

#[derive(Debug, Serialize)]
struct QuickstartReport {
    ledger_dir: String,
    actor_id: String,
    decision_id: String,
    query: QuickstartQueryReport,
}

#[derive(Debug, Serialize)]
struct QuickstartQueryReport {
    result_count: usize,
    total_matches: usize,
    truncated: bool,
    first_result_id: Option<String>,
}

// ---------------------------------------------------------------------------
// quality-scan command
// ---------------------------------------------------------------------------

fn run_quality_scan(cli: &Cli, args: &QualityScanArgs) -> Result<String> {
    use crate::linear::{
        file_findings, format_issue_description, format_issue_title, pending_findings, LinearClient,
    };

    let tenant_id = cli_tenant(cli)?;
    let ledger = open_ledger(cli)?;
    let graph = MemoryGraph::default();
    rebuild_graph_for_tenant(&ledger, &tenant_id, &graph)?;

    let request = ScanRequest {
        kinds: parse_kinds(&args.kinds).map_err(CliError::InvalidInput)?,
        limit: args.limit.clamp(1, 50),
        cursor: None,
        evidence_window_days: None,
    };
    // The connector's one read: the findings nobody has acknowledged. A finding it files is
    // acknowledged straight away, so a second run over an unchanged graph files nothing.
    let pending = pending_findings(&graph, &request, Utc::now())?;

    // Nothing to file is the steady state of a scan on a schedule. `--json` still answers with the
    // report, zero counts and no issues, so a pipe into `jq .filed` works on a clean ledger too;
    // only the text mode says it in a sentence.
    let nothing_pending = pending.data.findings.is_empty();
    if nothing_pending && !cli.json {
        return Ok("quality-scan: no unacknowledged finding — nothing to file".to_owned());
    }

    let base_url = args.hivemind_base_url.as_deref();

    let issues: Vec<serde_json::Value> = if nothing_pending {
        Vec::new()
    } else if args.dry_run {
        pending
            .data
            .findings
            .iter()
            .map(|scanned| {
                serde_json::json!({
                    "dry_run": true,
                    "finding_id": scanned.finding.finding_id,
                    "decision_id": scanned.finding.decision_id,
                    "kind": scanned.finding.kind.as_str(),
                    "title": format_issue_title(scanned),
                    "description": format_issue_description(scanned, base_url),
                })
            })
            .collect()
    } else {
        // Resolve Linear API key from env (not a CLI flag to avoid accidental exposure).
        let api_key = std::env::var("HIVEMIND_LINEAR_API_KEY").map_err(|_| {
            CliError::InvalidInput(
                "HIVEMIND_LINEAR_API_KEY is not set; pass --dry-run to preview without filing"
                    .to_owned(),
            )
        })?;
        let team_id = args.linear_team_id.clone().ok_or_else(|| {
            CliError::InvalidInput(
                "HIVEMIND_LINEAR_TEAM_ID is not set and --linear-team-id was not passed".to_owned(),
            )
        })?;
        let client = LinearClient::new(api_key);
        let commands = Commands::new_with_context(
            &ledger,
            CommandContext::new(tenant_id, fluent_write_provenance(&cli.actor)),
        );
        file_findings(
            pending.data.findings,
            &commands,
            &cli.actor,
            base_url,
            |title, description| client.create_issue(&team_id, title, description),
        )?
        .into_iter()
        .map(|filed| {
            serde_json::json!({
                "finding_id": filed.finding_id,
                "decision_id": filed.decision_id,
                "kind": filed.kind,
                "linear_identifier": filed.issue.identifier,
                "linear_url": filed.issue.url,
            })
        })
        .collect()
    };

    let output = serde_json::json!({
        "dry_run": args.dry_run,
        "scanned": pending.result_count,
        "filed": issues.len(),
        "truncated": pending.truncated,
        "issues": issues,
    });

    format_json_value(cli.json, &output)
}

// ---------------------------------------------------------------------------
// export subcommand
// ---------------------------------------------------------------------------

/// Directories the export owns besides the root `INDEX.md`: `projects/` today, and
/// `decisions/`, where the layout before decisions were grouped per project wrote its files.
const EXPORT_OWNED_DIRS: [&str; 2] = ["projects", "decisions"];

fn run_export(cli: &Cli, args: &ExportArgs) -> Result<String> {
    // Fails before any write when --out is a plain file, not a directory.
    if args.out.is_file() {
        return Err(CliError::InvalidInput(format!(
            "--out {} exists and is a file, not a directory",
            args.out.display()
        ))
        .into());
    }

    let tenant_id = cli_tenant(cli)?;
    let ledger = open_ledger(cli)?;
    let graph = MemoryGraph::default();
    rebuild_graph_for_tenant(&ledger, &tenant_id, &graph)?;

    let request = DecisionLogRequest {
        since: parse_query_datetime(args.since.as_deref(), "--since")?,
        topics: args.topic_keys.clone(),
        statuses: args
            .statuses
            .iter()
            .copied()
            .map(QueryDecisionStatus::as_decision_status)
            .collect(),
        project: args.project.clone(),
    };
    // The export replays this tenant's project registry and stamps its ledger offset, so it
    // reads through the tenant-scoped ledger like `project list` does, not the raw one.
    let scoped_ledger = TenantScopedLedger::new(&ledger, tenant_id);
    // Each decision's "Quality profile" section is the profile's own Markdown, read from the same
    // graph. The export is Layer 2 and cannot import the profile, so it is handed in here.
    let profile_section = |decision_id: &str| decision_log_section(&graph, decision_id);
    let export =
        match export_decision_log(&graph, &scoped_ledger, &request, Some(&profile_section))? {
            DecisionLogOutcome::Exported(export) => export,
            DecisionLogOutcome::ProjectNotFound { project } => {
                return format_export_output(cli.json, &ExportReport::NotFound { project });
            }
        };

    let summary = write_export_tree(&args.out, &export)?;

    let report = ExportReport::Exported {
        out_dir: args.out.display().to_string(),
        ledger_offset: export.ledger_offset,
        files_written: export.files.len(),
        files_removed: summary.removed,
    };
    format_export_output(cli.json, &report)
}

fn run_tenant(cli: &Cli, args: &TenantArgs) -> Result<String> {
    match &args.command {
        TenantCommand::Create(create_args) => run_tenant_create(cli, create_args),
    }
}

/// SQLite only — Postgres tenants are created through the server's
/// provisioning route (`POST /v1/tenants`), never the CLI.
fn run_tenant_create(cli: &Cli, args: &TenantCreateArgs) -> Result<String> {
    if cli
        .database_url
        .as_deref()
        .is_some_and(|url| !url.is_empty())
    {
        return Err(CliError::InvalidInput(
            "tenant create is SQLite-only; on Postgres, tenants are created through \
             the server's provisioning route (POST /v1/tenants)"
                .to_owned(),
        )
        .into());
    }

    let tenant_id = TenantId::new(args.tenant_id.trim().to_owned()).map_err(|error| {
        HivemindError::from(CliError::InvalidInput(format!(
            "tenant id is invalid: {error}"
        )))
    })?;

    let ledger = SqliteEventLedger::open(&cli.hivemind_dir)?;
    ledger.create_tenant(&tenant_id)?;

    if cli.json {
        format_json_value(
            true,
            &serde_json::json!({
                "tenant_id": tenant_id.as_str(),
                "hivemind_dir": cli.hivemind_dir.display().to_string(),
            }),
        )
    } else {
        Ok(format!(
            "Created tenant '{}' in {}",
            tenant_id.as_str(),
            cli.hivemind_dir.display()
        ))
    }
}

fn run_project(cli: &Cli, args: &ProjectArgs) -> Result<String> {
    match &args.command {
        ProjectCommand::Register(register_args) => run_project_register(cli, register_args),
        ProjectCommand::Link(link_args) => {
            run_project_link(cli, link_args, ProjectLinkAction::Link)
        }
        ProjectCommand::Unlink(link_args) => {
            run_project_link(cli, link_args, ProjectLinkAction::Unlink)
        }
        ProjectCommand::Anchor(anchor_args) => run_project_anchor(cli, anchor_args),
        ProjectCommand::DeclareTopic(declare_args) => run_project_declare_topic(cli, declare_args),
        ProjectCommand::List(list_args) => run_project_list(cli, list_args),
        ProjectCommand::Show(show_args) => run_project_show(cli, show_args),
        ProjectCommand::Decisions(decisions_args) => run_project_decisions(cli, decisions_args),
        ProjectCommand::Use(use_args) => run_project_use(cli, use_args),
    }
}

fn run_project_register(cli: &Cli, args: &ProjectRegisterArgs) -> Result<String> {
    let ledger = open_ledger(cli)?;
    let commands =
        Commands::new_with_context(&ledger, cli_command_context(cli, EventProvenance::cli())?);

    let event_id = commands.register_project(
        &cli.actor,
        &args.handle,
        args.display_name.as_deref(),
        args.purpose.as_deref(),
    )?;

    format_project_register_output(
        cli.json,
        &ProjectRegisterOutput {
            event_id,
            handle: args.handle.clone(),
            display_name: args.display_name.clone(),
            purpose: args.purpose.clone(),
        },
    )
}

#[derive(Clone, Copy)]
enum ProjectLinkAction {
    Link,
    Unlink,
}

fn run_project_link(
    cli: &Cli,
    args: &ProjectLinkArgs,
    action: ProjectLinkAction,
) -> Result<String> {
    let ledger = open_ledger(cli)?;
    let commands =
        Commands::new_with_context(&ledger, cli_command_context(cli, EventProvenance::cli())?);

    let kind = args.kind.as_project_link_kind();
    let event_id = match action {
        ProjectLinkAction::Link => commands.link_project(&cli.actor, &args.from, &args.to, kind)?,
        ProjectLinkAction::Unlink => {
            commands.unlink_project(&cli.actor, &args.from, &args.to, kind)?
        }
    };

    format_project_link_output(
        cli.json,
        &ProjectLinkOutput {
            event_id,
            from: args.from.clone(),
            to: args.to.clone(),
            kind: kind.as_str(),
        },
    )
}

fn run_project_anchor(cli: &Cli, args: &ProjectAnchorArgs) -> Result<String> {
    let ledger = open_ledger(cli)?;
    let commands =
        Commands::new_with_context(&ledger, cli_command_context(cli, EventProvenance::cli())?);

    let anchor_kind = args.anchor_kind.as_project_anchor_kind();
    let event_id = commands.anchor_project(&cli.actor, &args.handle, anchor_kind, &args.value)?;

    format_project_anchor_output(
        cli.json,
        &ProjectAnchorOutput {
            event_id,
            handle: args.handle.clone(),
            anchor_kind: anchor_kind.as_str(),
            value: args.value.clone(),
        },
    )
}

fn run_project_declare_topic(cli: &Cli, args: &ProjectDeclareTopicArgs) -> Result<String> {
    let ledger = open_ledger(cli)?;
    let commands =
        Commands::new_with_context(&ledger, cli_command_context(cli, EventProvenance::cli())?);

    let declarations = if args.in_use {
        commands.declare_topics_in_use(&cli.actor, &args.handle)?
    } else {
        args.topic_keys
            .iter()
            .map(|topic_key| commands.declare_project_topic(&cli.actor, &args.handle, topic_key))
            .collect::<Result<Vec<_>>>()?
    };

    format_project_declare_topic_output(
        cli.json,
        &ProjectDeclareTopicOutput {
            handle: args.handle.clone(),
            in_use: args.in_use,
            topics: declarations,
        },
    )
}

fn run_project_list(cli: &Cli, args: &ProjectListArgs) -> Result<String> {
    let tenant_id = cli_tenant(cli)?;
    let ledger = open_ledger(cli)?;
    let scoped_ledger = TenantScopedLedger::new(&ledger, tenant_id);

    let response = list_projects(
        &scoped_ledger,
        &ProjectListRequest {
            limit: args.limit,
            cursor: args.cursor.clone(),
        },
    )?;

    format_project_list_output(cli.json, &response)
}

fn run_project_show(cli: &Cli, args: &ProjectShowArgs) -> Result<String> {
    if args.current {
        if args.handle.is_some() {
            return Err(CliError::InvalidInput(
                "project show takes either a handle or --current, not both".to_owned(),
            )
            .into());
        }
        return run_project_show_current(cli);
    }

    let Some(handle) = args.handle.as_deref() else {
        return Err(CliError::InvalidInput(
            "project show requires a handle, or --current to show the current-project setting"
                .to_owned(),
        )
        .into());
    };

    let tenant_id = cli_tenant(cli)?;
    let ledger = open_ledger(cli)?;
    let scoped_ledger = TenantScopedLedger::new(&ledger, tenant_id);

    let response = get_project(&scoped_ledger, handle)?;

    format_project_show_output(cli.json, &response)
}

fn run_project_decisions(cli: &Cli, args: &ProjectDecisionsArgs) -> Result<String> {
    let tenant_id = cli_tenant(cli)?;
    let ledger = open_ledger(cli)?;
    let graph = MemoryGraph::default();
    rebuild_graph_for_tenant(&ledger, &tenant_id, &graph)?;

    let response = decisions_in_project(
        &graph,
        &ProjectDecisionsRequest {
            handle: args.handle.clone(),
            limit: args.limit,
            cursor: args.cursor.clone(),
        },
    )?;

    format_project_decisions_output(cli.json, &response)
}

fn run_project_show_current(cli: &Cli) -> Result<String> {
    let tenant_id = cli_tenant(cli)?;
    let store = CurrentProjectStore::new(&cli.hivemind_dir);
    let person = cli.person();
    let handle = store.get(tenant_id.as_str(), person.trim())?;

    format_current_project_output(
        cli.json,
        &CurrentProjectOutput {
            actor: person.trim().to_owned(),
            tenant: tenant_id.as_str().to_owned(),
            handle,
        },
    )
}

/// Set or clear the actor's current-project setting (`--hivemind-dir`, per
/// tenant + actor). This is deliberately NOT a ledger write -- Alex, choice
/// 5a: "local per machine, no fact in the record" -- so `--clear` never
/// needs the ledger at all. Setting a handle validates it against the
/// project registry first (same `get_project` read `project show` uses) so
/// the setting can never point at a typo (A3's refusal rule).
fn run_project_use(cli: &Cli, args: &ProjectUseArgs) -> Result<String> {
    let tenant_id = cli_tenant(cli)?;
    let person = cli.person();
    let actor = person.trim();
    let store = CurrentProjectStore::new(&cli.hivemind_dir);

    match (args.handle.as_deref(), args.clear) {
        (Some(_), true) => Err(CliError::InvalidInput(
            "project use takes either a handle or --clear, not both".to_owned(),
        )
        .into()),
        (None, false) => Err(CliError::InvalidInput(
            "project use requires a handle, or --clear to clear the current-project setting"
                .to_owned(),
        )
        .into()),
        (None, true) => {
            store.clear(tenant_id.as_str(), actor)?;
            format_current_project_output(
                cli.json,
                &CurrentProjectOutput {
                    actor: actor.to_owned(),
                    tenant: tenant_id.as_str().to_owned(),
                    handle: None,
                },
            )
        }
        (Some(handle), false) => {
            let handle = handle.trim();
            if handle.is_empty() {
                return Err(
                    CliError::InvalidInput("project handle must not be empty".to_owned()).into(),
                );
            }

            let ledger = open_ledger(cli)?;
            let scoped_ledger = TenantScopedLedger::new(&ledger, tenant_id.clone());
            let response = get_project(&scoped_ledger, handle)?;
            if matches!(response.data, ProjectOutcome::NotFound) {
                return Err(CliError::InvalidInput(format!(
                    "no project called '{handle}': run `hivemind project register {handle}` to register it"
                ))
                .into());
            }

            store.set(tenant_id.as_str(), actor, handle)?;
            format_current_project_output(
                cli.json,
                &CurrentProjectOutput {
                    actor: actor.to_owned(),
                    tenant: tenant_id.as_str().to_owned(),
                    handle: Some(handle.to_owned()),
                },
            )
        }
    }
}

struct ExportWriteSummary {
    removed: usize,
}

/// The only filesystem I/O in the export path. Writes every file in
/// `export.files` (relative path -> content) under `out_dir`, creating
/// directories as needed. The export owns `out_dir/INDEX.md` and every `.md`
/// file under `out_dir/projects/` — plus any left in `out_dir/decisions/` by
/// the layout before decisions were grouped per project. Any such file this
/// run did not produce is removed, so a narrower filter, a `--project` scope,
/// or a compacted ledger cannot leave stale files behind, and directories
/// the pruning empties go with them. Nothing else under
/// `out_dir` is touched: non-`.md` files anywhere in those trees, and every
/// file outside them.
fn write_export_tree(out_dir: &Path, export: &DecisionLogExport) -> Result<ExportWriteSummary> {
    let produced: BTreeSet<PathBuf> = export
        .files
        .keys()
        .map(|rel_path| out_dir.join(rel_path)) // ubs:ignore: rel_path keys come from decision_log::export_decision_log -- "INDEX.md" or "projects/..." built from handles restricted to [a-z0-9-], personal actor parts reduced to [A-Za-z0-9_-], and slug/id restricted to [a-z0-9-]/[A-Za-z0-9] (normalize_topic_key, short_id), so the join cannot leave out_dir
        .collect();

    let mut removed = 0usize;
    for owned in EXPORT_OWNED_DIRS {
        removed += prune_stale_markdown(&out_dir.join(owned), &produced)?;
    }

    for (rel_path, content) in &export.files {
        let full_path = out_dir.join(rel_path); // ubs:ignore: same restricted rel_path keys as above, so the join cannot leave out_dir
        if let Some(parent) = full_path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| {
                CliError::InvalidInput(format!(
                    "cannot create export output directory {}: {error}",
                    parent.display()
                ))
            })?;
        }
        std::fs::write(&full_path, content).map_err(|error| {
            CliError::InvalidInput(format!(
                "cannot write export file {}: {error}",
                full_path.display()
            ))
        })?;
    }

    Ok(ExportWriteSummary { removed })
}

/// Removes every `.md` file under `dir` that this run did not produce, then `dir` itself
/// and any directory below it that ends up empty (writing the new files recreates what they
/// need). Symlinks and non-`.md` files are left alone and keep their directory alive; a
/// missing `dir` has nothing to prune. Returns the number of files removed.
fn prune_stale_markdown(dir: &Path, produced: &BTreeSet<PathBuf>) -> Result<usize> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => {
            return Err(CliError::InvalidInput(format!(
                "cannot list export directory {}: {error}",
                dir.display()
            ))
            .into())
        }
    };

    let mut removed = 0usize;
    for entry in entries {
        let entry = entry.map_err(|error| {
            CliError::InvalidInput(format!(
                "cannot read export directory {}: {error}",
                dir.display()
            ))
        })?;
        let path = entry.path();
        // `DirEntry::file_type` does not follow symlinks, so a link is neither file nor dir.
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_dir() {
            removed += prune_stale_markdown(&path, produced)?;
        } else if kind.is_file()
            && path.extension().is_some_and(|ext| ext == "md")
            && !produced.contains(&path)
        {
            std::fs::remove_file(&path).map_err(|error| {
                CliError::InvalidInput(format!(
                    "cannot remove stale export file {}: {error}",
                    path.display()
                ))
            })?;
            removed += 1;
        }
    }

    if std::fs::read_dir(dir).is_ok_and(|mut rest| rest.next().is_none()) {
        std::fs::remove_dir(dir).map_err(|error| {
            CliError::InvalidInput(format!(
                "cannot remove emptied export directory {}: {error}",
                dir.display()
            ))
        })?;
    }
    Ok(removed)
}
