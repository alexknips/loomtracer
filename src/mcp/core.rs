//! Transport-agnostic core for MCP tools.
//!
//! Both the stdio transport (`mcp.rs`) and the MCP-over-HTTP transport
//! (`api/mcp_http.rs`) serve the same tool surface. Historically each did so
//! with its own ~90%-identical body, which drifted (see hivemind-whvd,
//! hivemind-q3rpv). This module is where that surface collapses to one
//! implementation per tool, consumed by both transports.
//!
//! Boundaries:
//!  * [`LedgerProvider`] is the only seam between the core and tenancy/auth.
//!    stdio implements it with a directory-based opener; HTTP implements it
//!    with a tenant-scoped resolver derived from validated WorkOS claims.
//!    The core never resolves tenancy or checks auth itself.
//!  * [`CoreError`] is the only error type the core produces. It carries no
//!    wire code — mapping it to a JSON-RPC error object (stdio) or an
//!    `(i32, String)` pair (HTTP) is entirely the adapter's job.
//!  * [`ToolOutput`] is the transport-neutral success payload. Rendering it
//!    to the MCP `content`/`structuredContent` wire shape is unchanged and
//!    stays with each adapter's existing renderer.
//!
//! Every tool in `tool_definitions()` lives here as an `Args::from_json` parser
//! plus a `core::<tool>` function, one pair per tool: a transport only
//! resolves who is calling and which ledger and graph to read, then calls the
//! pair. Two tools, [`HTTP_ONLY_TOOLS`], work the server's ingest queue and are
//! called by `POST /mcp` alone; stdio's `tools/list` leaves them out. A new
//! tool is added here, with a row in [`TOOL_KINDS`], and wired into both
//! dispatch tables.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use chrono::{DateTime, Duration, Utc};
use serde_json::{json, Map, Value};

use crate::commands::{
    insert_warnings_into, AcceptanceInput, CommandContext, Commands, DecisionPlacement,
    DecisionProposalInput, DeterminedProject, FloorInput, Grounding, Relay, SupersedeInput,
};
use crate::error::{CliError, CommandError, LoomtracerError};
use crate::events::{AckAction, CaptureItem, EventProvenance, ProjectSource, TenantId};
use crate::floor::FloorAnswers;
use crate::grounding::{
    premise_cycle_refusal, resolve_grounding_in, GroundingResolution, GroundingSpec,
    WIRE_GROUNDING_REFUSAL,
};
use crate::importance::{self, ImportanceRequest, DEFAULT_PAGE_SIZE};
use crate::ledger::{AnyLedger, EventLedger, TenantScopedLedger};
use crate::option_filler;
use crate::projector::{
    memory::{MemoryGraph, SharedGraph},
    rebuild_graph_for_tenant, GraphView,
};
use crate::quality_profile::{
    self, parse_kinds, ScanRequest, SuggestionsRequest, SCAN_DEFAULT_LIMIT,
};
use crate::queries::{
    annotate_resolution, context_next_cursor, derive_decision_status,
    get_changed_decisions as query_get_changed_decisions, get_compact_view,
    get_contested_decisions as query_get_contested_decisions, get_decision as query_get_decision,
    get_decision_brief as query_get_decision_brief,
    get_decision_context as query_get_decision_context, get_decision_context_candidates,
    get_decision_neighborhood as query_get_decision_neighborhood, get_decision_quality_candidates,
    get_decision_timeline as query_get_decision_timeline,
    get_open_why_decisions as query_get_open_why_decisions, get_recent_decisions,
    get_reexamine_items as query_get_reexamine_items,
    get_relevant_decisions as query_get_relevant_decisions,
    get_supersession_chain as query_get_supersession_chain,
    get_waiting_requests as query_get_waiting_requests, misfiled_next_cursor, outcome_next_cursor,
    outcome_page_len, require_registered_project, resolve_decision_by_description,
    resolve_decision_for_reading, scan_misfiled_decisions as query_scan_misfiled_decisions,
    search_decisions_any, upstream_of, ChangedDecisionsRequest, ContestedDecisionsRequest,
    DecisionContextRequest, DecisionQualityCandidatesRequest, DecisionStatus,
    FailureAttributionRequest, MisfiledScanRequest, NeighborhoodRequest, OpenWhyRequest,
    QueryContext, QueryResponse, RecentDecisionFilterRequest, RecentDecisionsRequest,
    ReexamineRequest, ResolveOutcome, ResolvedCandidate, SearchDecisionRequest, SituationalRequest,
    WaitingRequestsRequest, DEFAULT_REEXAMINE_PAGE_SIZE,
};
use crate::reaffirmation::{reaffirm, ReaffirmCall, ReaffirmOutcome};
use crate::summarize::{
    RecallRequest, SummarizeMode, SummarizeRequest, RECALL_DEFAULT_LIMIT, RECALL_MAX_LIMIT,
};
use crate::supersession_link::{SupersessionLinkCall, SupersessionLinkOutcome};

use super::args::{
    optional_bool, optional_bool_or, optional_datetime, optional_option_labels, optional_string,
    optional_string_array, optional_usize, require_string, require_string_array, topic_keys_arg,
};

// ---------------------------------------------------------------------------
// Error shape
// ---------------------------------------------------------------------------

/// Transport-neutral error from a tool core.
///
/// No JSON-RPC code and no HTTP status live here on purpose — those are wire
/// concerns, and each transport already has its own error envelope. Adapters
/// map each variant to their own wire shape.
///
/// `capture_decision` produced only the first variant; per Alex's
/// 2026-09-20 call, a free-text description that matches nothing is a
/// success outcome (`{outcome: "not_found"}`, see [`ResolvedTarget::NotFound`]),
/// not an error, so `resolve_target` never needed its own `CoreError`
/// variant. whvd.1's note that later tools add their own variant when they
/// need one still stands — none has, yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CoreError {
    /// Caller-supplied arguments failed validation.
    InvalidArgument(String),
    /// Ledger I/O or any other internal failure.
    Internal(String),
}

impl CoreError {
    pub(crate) fn into_message(self) -> String {
        match self {
            CoreError::InvalidArgument(m) | CoreError::Internal(m) => m,
        }
    }
}

impl std::fmt::Display for CoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CoreError::InvalidArgument(m) | CoreError::Internal(m) => {
                write!(f, "{m}")
            }
        }
    }
}

impl From<LoomtracerError> for CoreError {
    fn from(error: LoomtracerError) -> Self {
        match &error {
            LoomtracerError::Command(CommandError::Validation(_))
            | LoomtracerError::Cli(CliError::InvalidInput(_)) => {
                CoreError::InvalidArgument(error.to_string())
            }
            _ => CoreError::Internal(error.to_string()),
        }
    }
}

/// The shared JSON-RPC arg helpers in `args.rs` return the raw `(code,
/// message)` pair. Every error they raise is a validation failure, so the
/// conversion is unconditional — this lets `?` bridge straight from those
/// helpers into `CoreError` without per-call-site `map_err`.
impl From<(i32, String)> for CoreError {
    fn from((_, message): (i32, String)) -> Self {
        CoreError::InvalidArgument(message)
    }
}

// ---------------------------------------------------------------------------
// Ledger acquisition seam
// ---------------------------------------------------------------------------

/// A ledger already scoped to the right tenant, handed to the core by its
/// transport. The core only ever sees the result of tenant resolution, never
/// how it happened.
pub(crate) struct LedgerHandle<L: EventLedger> {
    pub(crate) ledger: L,
    pub(crate) tenant_id: TenantId,
    /// Where a call that needs the tenant's graph gets it, when the transport keeps one current.
    /// `None` replays the whole ledger into a fresh graph: right for a transport that opens its
    /// ledger per call (stdio), and a pass over every transcript on a ledger the size of the city
    /// cell's. The HTTP server supplies its cache, which catches up from its last offset, so a
    /// write that only looks a decision up no longer replays the ledger (hivemind-f38ka).
    pub(crate) graph_source: Option<GraphSource<L>>,
}

/// A transport's way of handing the core the tenant's projected graph, current as of the call.
pub(crate) type GraphSource<L> = Box<dyn Fn(&L, &TenantId) -> crate::Result<Arc<MemoryGraph>>>;

impl<L: EventLedger> LedgerHandle<L> {
    /// The tenant's graph as the ledger stands now, including anything this call just wrote.
    /// Read-only: it may be the graph the server's cache hands every other reader.
    pub(crate) fn graph(&self) -> crate::Result<SharedGraph> {
        let graph = match &self.graph_source {
            Some(source) => source(&self.ledger, &self.tenant_id)?,
            None => {
                let graph = MemoryGraph::default();
                rebuild_graph_for_tenant(&self.ledger, &self.tenant_id, &graph)?;
                Arc::new(graph)
            }
        };
        Ok(SharedGraph::new(graph))
    }
}

/// Transport-specific ledger acquisition, injected into the core.
///
/// stdio implements this with a directory-based opener; HTTP implements it
/// with a tenant-scoped resolver derived from validated WorkOS claims. Auth
/// stays entirely on the HTTP side of that boundary — this trait is called
/// only after the adapter has already authenticated the caller.
pub(crate) trait LedgerProvider {
    type Ledger: EventLedger;

    fn ledger(&self) -> Result<LedgerHandle<Self::Ledger>, CoreError>;
}

// ---------------------------------------------------------------------------
// Success payload
// ---------------------------------------------------------------------------

/// Transport-neutral payload from a successful core call. Each transport
/// renders this into its own `content`/`structuredContent` wire shape
/// exactly as it did for the plain `Value` payloads before migration.
#[derive(Debug, Clone)]
pub(crate) struct ToolOutput(pub(crate) Value);

impl ToolOutput {
    pub(crate) fn into_value(self) -> Value {
        self.0
    }
}

/// What a tool does to the ledger. Declared once per tool in [`TOOL_KINDS`]; everything that
/// treats reads and writes differently asks the table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ToolKind {
    /// Appends events. Once started it runs to its result: cutting it off would answer
    /// "failed" about an event that is recorded (hivemind-s9cx).
    Write,
    /// Reads decisions back, so its reply carries the unreadable-rows notice (`read_notice`).
    ReadDecisions,
    /// Reads, but not decisions: the classification queue.
    ReadOther,
}

impl ToolKind {
    /// A read has nothing to commit, so the server may stop waiting for it.
    pub(crate) fn is_read(self) -> bool {
        !matches!(self, ToolKind::Write)
    }
}

/// The kind of every tool in `tool_definitions`. A tool added without a row here fails
/// `every_tool_has_a_kind`, instead of silently running unbounded or without the notice.
pub(crate) const TOOL_KINDS: &[(&str, ToolKind)] = &[
    ("capture_decision", ToolKind::Write),
    ("capture_evidence", ToolKind::Write),
    ("capture_hypothesis", ToolKind::Write),
    ("accept_decision", ToolKind::Write),
    ("disagree_decision", ToolKind::Write),
    ("dispute_attribution", ToolKind::Write),
    ("supersede_decision", ToolKind::Write),
    ("link_supersession", ToolKind::Write),
    ("move_decision", ToolKind::Write),
    ("retitle_decision", ToolKind::Write),
    ("acknowledge_suggestion", ToolKind::Write),
    ("ground_decision", ToolKind::Write),
    ("reaffirm_decision", ToolKind::Write),
    ("request_decision", ToolKind::Write),
    ("classify_queue_submit", ToolKind::Write),
    ("classify_queue_list", ToolKind::ReadOther),
    ("get_waiting_requests", ToolKind::ReadDecisions),
    ("get_contested_decisions", ToolKind::ReadDecisions),
    ("get_open_why_decisions", ToolKind::ReadDecisions),
    ("get_reexamine_items", ToolKind::ReadDecisions),
    ("get_changed_decisions", ToolKind::ReadDecisions),
    ("get_decision", ToolKind::ReadDecisions),
    ("get_decision_outcome", ToolKind::ReadDecisions),
    ("decision_quality_candidates", ToolKind::ReadDecisions),
    ("get_decision_context", ToolKind::ReadDecisions),
    ("decision_context_candidates", ToolKind::ReadDecisions),
    ("score_decision", ToolKind::ReadDecisions),
    ("scan_decision_quality", ToolKind::ReadDecisions),
    ("get_suggestions", ToolKind::ReadDecisions),
    ("rank_decisions_by_importance", ToolKind::ReadDecisions),
    ("scan_misfiled_decisions", ToolKind::ReadDecisions),
    ("analyze_failure_modes", ToolKind::ReadDecisions),
    ("get_relevant_decisions", ToolKind::ReadDecisions),
    ("get_situational_decisions", ToolKind::ReadDecisions),
    ("get_supersession_chain", ToolKind::ReadDecisions),
    ("get_decision_neighborhood", ToolKind::ReadDecisions),
    ("search_decisions", ToolKind::ReadDecisions),
    ("recall_decisions", ToolKind::ReadDecisions),
    ("recent_decisions", ToolKind::ReadDecisions),
    ("dump_graph", ToolKind::ReadDecisions),
    ("loomtracer_compact_view", ToolKind::ReadDecisions),
    ("summarize_decisions", ToolKind::ReadDecisions),
];

/// The tools only `POST /mcp` serves: they work the server's ingest queue, which a local stdio
/// ledger has no workers for (hivemind-zdsh.18). Stdio's `tools/list` leaves them out, so it
/// never advertises a tool it would answer "unknown tool" to.
pub(crate) const HTTP_ONLY_TOOLS: &[&str] = &["classify_queue_list", "classify_queue_submit"];

/// True for a tool stdio does not serve.
pub(crate) fn tool_is_http_only(name: &str) -> bool {
    HTTP_ONLY_TOOLS.contains(&name)
}

/// The kind of the named tool, or `None` for a name that is not a tool.
pub(crate) fn tool_kind(name: &str) -> Option<ToolKind> {
    TOOL_KINDS
        .iter()
        .find(|(tool, _)| *tool == name)
        .map(|(_, kind)| *kind)
}

/// True for every tool that reads the ledger's decisions back.
pub(crate) fn tool_reads_decisions(name: &str) -> bool {
    tool_kind(name) == Some(ToolKind::ReadDecisions)
}

/// The top-level argument names each tool's `inputSchema.properties` lists, by tool name.
fn tool_argument_names() -> &'static HashMap<String, Vec<String>> {
    static NAMES: OnceLock<HashMap<String, Vec<String>>> = OnceLock::new();
    NAMES.get_or_init(|| {
        super::tool_definitions()
            .iter()
            .filter_map(|tool| {
                let name = tool.get("name")?.as_str()?;
                let listed = tool
                    .pointer("/inputSchema/properties")
                    .and_then(Value::as_object)
                    .map(|properties| properties.keys().cloned().collect())
                    .unwrap_or_default();
                Some((name.to_owned(), listed))
            })
            .collect()
    })
}

/// Refuse a `tools/call` that sends an argument the tool's schema does not list. A parser reads
/// the keys it knows and skips the rest, so without this check a misspelt or wrong-tool argument
/// (`query` for `q`, `decided_by` on `link_supersession`) runs as if it was never sent and the
/// caller believes it took effect. Both transports call this before dispatch; a name that is not
/// a tool passes, and the dispatch answers "unknown tool". Top-level keys only: the nested
/// objects that must be closed say so in their own schema.
pub(crate) fn reject_unknown_arguments(
    tool: &str,
    arguments: &Map<String, Value>,
) -> Result<(), CoreError> {
    let Some(listed) = tool_argument_names().get(tool) else {
        return Ok(());
    };
    let unknown: Vec<String> = arguments
        .keys()
        .filter(|key| !listed.contains(key))
        .map(|key| format!("`{key}`"))
        .collect();
    if unknown.is_empty() {
        return Ok(());
    }
    let takes = if listed.is_empty() {
        "takes no arguments".to_owned()
    } else {
        format!("takes: {}", listed.join(", "))
    };
    Err(CoreError::InvalidArgument(format!(
        "unknown argument{} {}; {tool} {takes}",
        if unknown.len() == 1 { "" } else { "s" },
        unknown.join(", "),
    )))
}

/// What `dispute_attribution` tells an agent that calls it without the person (hivemind-9vrok):
/// the tool records a person's dispute through the agent, so it needs the person and their
/// words, and an agent's own disagreement is another tool. A decider agent that disputes its own
/// acceptance sends `reason` alone (hivemind-53dmq).
const DISPUTE_ATTRIBUTION_NEEDS_A_PERSON: &str =
    "dispute_attribution records, for a person, that the call was never theirs: pass \
     `disputed_by` (the person, `human:<name>`) and `quote` (their verbatim words). If the \
     decision names you as its decider and the call was never yours, send your own words as \
     `reason`, with nothing else. To record that you disagree with the decision yourself, use \
     `disagree_decision` with your `reason`; to replace it, `capture_decision` then \
     `link_supersession`";

/// What `dispute_attribution` tells an agent whose own `reason` was refused because the decision
/// does not name it as a decider: the dispute is for the decider's own acceptance only, so it
/// gets the paths it has instead, as tools (hivemind-53dmq). It does not offer `reason` again.
fn dispute_attribution_not_a_decider(actor_id: &str, decision_id: &str) -> CoreError {
    CoreError::InvalidArgument(format!(
        "{actor_id} is not recorded as a decider of decision {decision_id}, so it cannot dispute \
         that attribution: only an actor the decision names as its decider can say the call was \
         not theirs, and only for itself. If a person told you the call was never theirs, pass \
         `disputed_by` (the person, `human:<name>`) and their verbatim words as `quote`. To \
         record that you disagree with the decision yourself, use `disagree_decision` with your \
         `reason`; to replace it, `capture_decision` then `link_supersession`"
    ))
}

/// The tools that append events to the ledger, in [`TOOL_KINDS`] order. The site's reference
/// generator sorts its tool pages into write and read tools with this, so what the page calls a
/// write tool is what the server runs as one.
pub fn write_tool_names() -> Vec<&'static str> {
    TOOL_KINDS
        .iter()
        .filter(|(_, kind)| *kind == ToolKind::Write)
        .map(|(name, _)| *name)
        .collect()
}

// ---------------------------------------------------------------------------
// project / project_source (capture_decision, supersede_decision)
// ---------------------------------------------------------------------------

/// Parse the `project` / `project_source` pair the two write tools share. The handle is
/// whatever the caller determined; Loomtracer only validates it (registered, not a reserved
/// personal address), never infers one. `project_source` needs a `project` to describe.
fn project_args(
    args: &Map<String, Value>,
) -> Result<(Option<String>, Option<ProjectSource>), CoreError> {
    let project = optional_string(args, "project")?.map(|handle| handle.trim().to_owned());
    let project_source = optional_string(args, "project_source")?
        .map(|value| {
            ProjectSource::parse(&value).ok_or_else(|| {
                CoreError::InvalidArgument(format!(
                    "`project_source` must be one of stated, folder_marker, rig, current_project, job: {value}"
                ))
            })
        })
        .transpose()?;
    if project_source.is_some() && project.is_none() {
        return Err(CoreError::InvalidArgument(
            "`project_source` requires `project`".to_owned(),
        ));
    }
    Ok((project, project_source))
}

fn determined_project<'a>(
    project: Option<&'a str>,
    project_source: Option<ProjectSource>,
) -> Option<DeterminedProject<'a>> {
    project.map(|handle| DeterminedProject {
        handle,
        source: project_source.unwrap_or(ProjectSource::Stated),
    })
}

/// Name the project a write landed in: `project` and `project_source` always, and on
/// personal fallback the "saved to your personal project" sentence, so it is never silent.
fn insert_placement(reply: &mut Value, placement: &DecisionPlacement) {
    if let Some(reply) = reply.as_object_mut() {
        reply.insert("project".to_owned(), json!(placement.project));
        reply.insert("project_source".to_owned(), json!(placement.project_source));
        if let Some(notice) = placement.notice() {
            reply.insert("project_notice".to_owned(), json!(notice));
        }
        if !placement.declared_topics.is_empty() {
            reply.insert(
                "declared_topics".to_owned(),
                json!(placement.declared_topics),
            );
        }
    }
}

// ---------------------------------------------------------------------------
// capture_decision
// ---------------------------------------------------------------------------

/// One parsed, validated option from `capture_decision`'s `options` array.
pub(crate) struct CaptureOptionArg {
    pub(crate) label: String,
    pub(crate) description: String,
}

/// Parsed, validated arguments for the `capture_decision` tool. Shared by
/// both transports so argument-shape validation — and its error text — can
/// never drift between them again (it already had once: stdio and HTTP
/// disagreed on the empty-label message before this module existed).
pub(crate) struct CaptureDecisionArgs {
    pub(crate) actor_id: String,
    pub(crate) title: String,
    pub(crate) rationale: String,
    pub(crate) topic_keys: Vec<String>,
    pub(crate) options: Vec<CaptureOptionArg>,
    pub(crate) chosen_option_label: Option<String>,
    /// Actor who actually made the decision, when it differs from `actor_id` (the recording
    /// actor — an agent scribing a decision a human made, for example). Requires
    /// `chosen_option_label`. See `decided_by` on `DecisionProposalInput`. Mutually exclusive
    /// with `still_proposed`.
    pub(crate) decided_by: Option<String>,
    /// The human whose delegated scope this decision falls within, when `actor_id` (an
    /// agent) decided it for itself. Requires `chosen_option_label`. See `delegated_by` on
    /// `DecisionProposalInput`.
    pub(crate) delegated_by: Option<String>,
    /// Keep the decision at `proposed` even though `chosen_option_label` is set. See
    /// `still_proposed` on `DecisionProposalInput`.
    pub(crate) still_proposed: bool,
    /// What the decision rests on: the `grounding` array plus the deprecated `hypothesis_ids` /
    /// `evidence_ids` aliases. Never empty — see [`GroundingSpec::from_wire`].
    pub(crate) grounding: GroundingSpec,
    /// What the decision binds and what undoing it costs: the `binds` and `undo_cost` objects.
    /// Both required — see [`FloorAnswers::from_wire`].
    pub(crate) floor: FloorAnswers,
    /// Confidence in the decider's own words: low | medium | high. Validated by the commands.
    pub(crate) expressed_confidence: Option<String>,
    /// Verbatim words of the decider, self-contained. Requires `question`. See `quote` on
    /// `DecisionProposalInput`.
    pub(crate) quote: Option<String>,
    /// The question this decision answers. Required by `quote`; otherwise optional. Resolved to
    /// a `Question` node. See `question` on `DecisionProposalInput`.
    pub(crate) question: Option<String>,
    /// The id of an existing `request_decision` request this decision answers: resolves to that
    /// request's question and links this decision to it, exactly as `question` would, without
    /// repeating the words. Refused when the request does not exist, or together with
    /// `question`. Distinct from `ground_decision`'s `answers`, which takes the question's own
    /// text, not a request id.
    pub(crate) answers: Option<String>,
    /// Registered project handle to file the decision under. See [`project_args`].
    pub(crate) project: Option<String>,
    /// How `project` was determined; `stated` when omitted. Requires `project`.
    pub(crate) project_source: Option<ProjectSource>,
    /// Topic keys this capture adds to its project's vocabulary; each must be one of
    /// `topic_keys`. See `Commands::declaring_topics`.
    pub(crate) declare_topics: Vec<String>,
}

/// Parse and require the wire grounding shared by `capture_decision`, `supersede_decision` and
/// `ground_decision`: the `grounding` array (required, minItems 1) plus the deprecated
/// `hypothesis_ids` / `evidence_ids` aliases, which map into the same spec. A call that names
/// nothing it rests on is refused with `refusal` before anything is resolved or written.
fn require_wire_grounding(
    args: &Map<String, Value>,
    refusal: &str,
) -> Result<GroundingSpec, CoreError> {
    let spec = wire_grounding(args)?;
    if spec.is_empty() {
        return Err(CoreError::InvalidArgument(refusal.to_owned()));
    }
    Ok(spec)
}

/// The decision floor's two answers from the wire `binds` and `undo_cost` objects, shared with
/// REST (see [`FloorAnswers::from_wire`]). A call that leaves either out is refused before
/// anything is resolved or written.
fn wire_floor(args: &Map<String, Value>) -> Result<FloorAnswers, CoreError> {
    FloorAnswers::from_wire(args.get("binds"), args.get("undo_cost"))
        .map_err(CoreError::InvalidArgument)
}

/// The wire grounding, possibly naming nothing: [`require_wire_grounding`] without the refusal,
/// for a call (`ground_decision` with `answers`) that may legitimately carry none.
fn wire_grounding(args: &Map<String, Value>) -> Result<GroundingSpec, CoreError> {
    let items = match args.get("grounding") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(items)) => items.clone(),
        Some(_) => {
            return Err(CoreError::InvalidArgument(
                "`grounding` must be an array".to_owned(),
            ))
        }
    };
    let hypothesis_ids = optional_string_array(args, "hypothesis_ids")?;
    let evidence_ids = optional_string_array(args, "evidence_ids")?;
    GroundingSpec::from_wire(&items, &hypothesis_ids, &evidence_ids)
        .map_err(CoreError::InvalidArgument)
}

impl CaptureDecisionArgs {
    pub(crate) fn from_json(
        args: &Map<String, Value>,
        actor_id: String,
    ) -> Result<Self, CoreError> {
        let title = require_string(args, "title")?;
        let rationale = require_string(args, "rationale")?;
        let topic_keys = topic_keys_arg(args, "topic_keys", true)?;
        if topic_keys.is_empty() {
            return Err(CoreError::InvalidArgument(
                "topic_keys must not be empty".to_owned(),
            ));
        }

        let options_value = args
            .get("options")
            .cloned()
            .ok_or_else(|| CoreError::InvalidArgument("missing `options`".to_owned()))?;
        let options_array = match options_value {
            Value::Array(items) => items,
            _ => {
                return Err(CoreError::InvalidArgument(
                    "`options` must be an array".to_owned(),
                ))
            }
        };
        if options_array.is_empty() {
            return Err(CoreError::InvalidArgument(
                "options must not be empty".to_owned(),
            ));
        }

        let mut options = Vec::with_capacity(options_array.len());
        for (index, option) in options_array.into_iter().enumerate() {
            let option_obj = match option {
                Value::Object(map) => map,
                _ => {
                    return Err(CoreError::InvalidArgument(format!(
                        "options[{index}] must be an object"
                    )))
                }
            };
            let label = option_obj
                .get("label")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| {
                    CoreError::InvalidArgument(format!(
                        "options[{index}].label must be a non-empty string"
                    ))
                })?
                .to_owned();
            let description = option_obj
                .get("description")
                .and_then(Value::as_str)
                .filter(|s| !s.trim().is_empty())
                .map(str::to_owned)
                .unwrap_or_else(|| option_filler::mcp(&label));
            options.push(CaptureOptionArg { label, description });
        }

        let chosen_option_label = optional_string(args, "chosen_option_label")?;
        let decided_by = optional_string(args, "decided_by")?;
        let delegated_by = optional_string(args, "delegated_by")?;
        let still_proposed = optional_bool(args, "still_proposed")?;

        if decided_by.is_some() && chosen_option_label.is_none() {
            return Err(CoreError::InvalidArgument(
                "decided_by requires chosen_option_label".to_owned(),
            ));
        }
        if delegated_by.is_some() && chosen_option_label.is_none() {
            return Err(CoreError::InvalidArgument(
                "delegated_by requires chosen_option_label".to_owned(),
            ));
        }

        let quote = optional_string(args, "quote")?;
        let question = optional_string(args, "question")?;
        let answers = optional_string(args, "answers")?;
        let (project, project_source) = project_args(args)?;

        if quote.is_some() && question.is_none() {
            return Err(CoreError::InvalidArgument(
                "quote requires question — a verbatim answer needs the question it answers spelled out, not a bare reference like '1a' into an external list".to_owned(),
            ));
        }
        if answers.is_some() && question.is_some() {
            return Err(CoreError::InvalidArgument(
                "answers and question are mutually exclusive: the request already names its question".to_owned(),
            ));
        }

        let grounding = require_wire_grounding(args, WIRE_GROUNDING_REFUSAL)?;
        let floor = wire_floor(args)?;
        let expressed_confidence = optional_string(args, "expressed_confidence")?;

        Ok(Self {
            actor_id,
            title,
            rationale,
            topic_keys,
            options,
            chosen_option_label,
            decided_by,
            delegated_by,
            still_proposed,
            grounding,
            floor,
            expressed_confidence,
            quote,
            question,
            answers,
            project,
            project_source,
            declare_topics: optional_string_array(args, "declare_topics")?,
        })
    }
}

/// The migrated core for the `capture_decision` MCP tool: one implementation
/// consumed by both transports.
pub(crate) fn capture_decision<P: LedgerProvider>(
    provider: &P,
    args: CaptureDecisionArgs,
) -> Result<ToolOutput, CoreError> {
    let handle = provider.ledger()?;
    // Resolve every named premise before anything is written: an ambiguous or unmatched one is
    // returned as data (like a fluent tool's target) and no event is appended.
    let resolved =
        match resolve_grounding_in(args.grounding, || handle.graph()).map_err(CoreError::from)? {
            GroundingResolution::Ready(resolved) => resolved,
            GroundingResolution::Unresolved(unresolved) => {
                return Ok(ToolOutput(unresolved.envelope()));
            }
        };
    let commands = Commands::new_with_context(
        &handle.ledger,
        CommandContext::new(
            handle.tenant_id,
            EventProvenance::agent(args.actor_id.clone()),
        ),
    )
    .declaring_topics(&args.declare_topics);

    let mut option_ids: Vec<String> = Vec::with_capacity(args.options.len());
    let mut option_labels: Vec<String> = Vec::with_capacity(args.options.len());
    let mut chosen_option_id: Option<String> = None;
    // Consumes args.options (moved, not iterated by reference) so each label is
    // pushed by ownership transfer instead of a per-iteration clone; args.options
    // is not read again after this loop.
    for option in args.options {
        let option_id = commands
            .record_option(&args.actor_id, &option.label, &option.description)
            .map_err(CoreError::from)?;
        // ubs:ignore: == compares option labels (user-visible strings), not secrets
        if args.chosen_option_label.as_deref() == Some(option.label.as_str()) {
            chosen_option_id = Some(option_id.clone());
        }
        option_ids.push(option_id);
        option_labels.push(option.label);
    }
    if args.chosen_option_label.is_some() && chosen_option_id.is_none() {
        return Err(CoreError::InvalidArgument(
            "chosen_option_label must match one of the supplied option labels".to_owned(),
        ));
    }

    let question = commands
        .resolve_answers_request(args.answers.as_deref(), args.question.as_deref())
        .map_err(CoreError::from)?;

    let proposal = commands
        .propose_grounded_decision(
            DecisionProposalInput {
                // The plan carries every id; see `propose_grounded_decision`.
                grounding: Grounding::NotAsked,
                hypothesis_ids: &[],
                evidence_ids: &[],
                expressed_confidence: args.expressed_confidence.as_deref(),
                project: determined_project(args.project.as_deref(), args.project_source),
                actor_id: &args.actor_id,
                title: &args.title,
                rationale: &args.rationale,
                topic_keys: &args.topic_keys,
                option_ids: &option_ids,
                option_labels: &option_labels,
                chosen_option_id: chosen_option_id.as_deref(),
                decided_by: args.decided_by.as_deref(),
                delegated_by: args.delegated_by.as_deref(),
                still_proposed: args.still_proposed,
                quote: args.quote.as_deref(),
                question: question.as_deref(),
                floor: FloorInput::Stated(&args.floor),
            },
            &resolved.plan,
        )
        .map_err(CoreError::from)?;

    let mut reply = json!({
        "decision_id": proposal.decision_id,
        "option_ids": option_ids,
        "chosen_option_id": chosen_option_id,
        "decided_by": args.decided_by,
        "delegated_by": args.delegated_by,
        "still_proposed": args.still_proposed,
        "rests_on": resolved.label(proposal.rests_on),
        "premise_stale": proposal.premise_stale,
    });
    if !proposal.cited_not_linked.is_empty() {
        if let Some(reply) = reply.as_object_mut() {
            reply.insert(
                "cited_not_linked".to_owned(),
                json!(proposal.cited_not_linked),
            );
        }
    }
    insert_warnings_into(&proposal.warnings, &mut reply);
    if let Some(question) = proposal.question {
        reply["question_id"] = json!(question.question_id);
    }
    args.floor.reply().insert_into(&mut reply);
    insert_placement(&mut reply, &proposal.placement);
    Ok(ToolOutput(reply))
}

// ---------------------------------------------------------------------------
// request_decision
// ---------------------------------------------------------------------------

/// Parsed, validated arguments for the `request_decision` tool (hivemind-bbnw.4).
pub(crate) struct RequestDecisionArgs {
    pub(crate) actor_id: String,
    pub(crate) text: String,
}

impl RequestDecisionArgs {
    pub(crate) fn from_json(
        args: &Map<String, Value>,
        actor_id: String,
    ) -> Result<Self, CoreError> {
        let text = require_string(args, "text")?;
        Ok(Self { actor_id, text })
    }
}

/// The migrated core for the `request_decision` MCP tool: records an explicit ask, before any
/// decision answers it. Resolved and created exactly like `capture_decision`'s `question` (a
/// match on the normalized text is reused, otherwise a `Question` node is created), plus a
/// `question.asked` event recording who asked and when — never suppressed as a duplicate, so
/// asking the same question again is a second, independent request (hivemind-bbnw.4).
pub(crate) fn request_decision<P: LedgerProvider>(
    provider: &P,
    args: RequestDecisionArgs,
) -> Result<ToolOutput, CoreError> {
    let handle = provider.ledger()?;
    let commands = Commands::new_with_context(
        &handle.ledger,
        CommandContext::new(
            handle.tenant_id,
            EventProvenance::agent(args.actor_id.clone()),
        ),
    );
    let plan = commands.plan_ask(&args.text).map_err(CoreError::from)?;
    let recorded = commands
        .record_ask(&args.actor_id, &plan)
        .map_err(CoreError::from)?;
    Ok(ToolOutput(json!({
        "request_id": recorded.request_id,
        "question_id": recorded.question_id,
        "reused": recorded.reused,
    })))
}

// ---------------------------------------------------------------------------
// get_situational_decisions
// ---------------------------------------------------------------------------

/// Parsed, validated arguments for the `get_situational_decisions` tool. No
/// resolver involved: situational takes paths, not a free-text description,
/// so there is no id/description ambiguity to settle before calling the
/// query layer.
pub(crate) struct GetSituationalDecisionsArgs {
    pub(crate) paths: Vec<String>,
    pub(crate) since_offset: Option<u64>,
    pub(crate) since_timestamp: Option<chrono::DateTime<chrono::Utc>>,
    pub(crate) limit: usize,
    pub(crate) cursor: Option<String>,
    pub(crate) project: Option<String>,
}

impl GetSituationalDecisionsArgs {
    pub(crate) fn from_json(args: &Map<String, Value>) -> Result<Self, CoreError> {
        let paths = require_string_array(args, "paths")?;
        let since_offset = optional_usize(args, "since_offset")?.map(|value| value as u64);
        let since_timestamp = optional_datetime(args, "since_timestamp")?;
        let limit = optional_usize(args, "limit")?.unwrap_or(0);
        let cursor = optional_string(args, "cursor")?;
        let project = optional_string(args, "project")?;
        Ok(Self {
            paths,
            since_offset,
            since_timestamp,
            limit,
            cursor,
            project,
        })
    }
}

// ---------------------------------------------------------------------------
// resolve_target: the shared fluent (no-id) target resolver
// ---------------------------------------------------------------------------
//
// The CLI's equivalent (`resolve_fluent_target`, `src/cli/run/mod.rs`) supports
// `--pick`/`#N` because it has a local, single-user session to cache candidates
// in. MCP has neither: stdio is stateless per call and HTTP has no session
// directory, so there is no disambiguation shortcut here — an ambiguous or
// not-found description is returned to the caller as a success envelope, and
// the caller re-calls with `decision_id` once it knows one.

/// Shared "one of `<selector_field>` or `description` is required" message.
/// Both transports raise this exact text so they can never drift on it, the
/// same discipline `CaptureDecisionArgs` already applies to its own
/// validation. `selector_field` is the caller's id parameter name —
/// `decision_id` for `get_decision_neighborhood`, `old_decision_id` for
/// `supersede_decision` — so the message always names the field the caller
/// actually sent.
fn missing_selector_message(selector_field: &str) -> String {
    format!("one of `{selector_field}` or `description` is required")
}

/// Outcome of resolving a fluent MCP tool's target.
pub(crate) enum ResolvedTarget {
    /// A concrete decision id to proceed with — either the caller supplied it
    /// directly, or resolution found exactly one best-tier match.
    Id(String),
    /// Resolution matched more than one decision at the best tier. This is a
    /// successful outcome (not an error): the caller returns it as-is.
    Ambiguous(ToolOutput),
    /// The description matched nothing. This is also a successful outcome —
    /// `{outcome: "not_found"}` — not an error, per Alex's 2026-09-20 call:
    /// MCP has no session to retry a bare id against, but the caller can
    /// still branch on `data.outcome` without needing a JSON-RPC error path.
    NotFound(ToolOutput),
}

/// Resolve a fluent tool's target: `id` bypasses resolution entirely
/// (unvalidated, matching the CLI's `--id` escape hatch byte-for-byte);
/// otherwise `description` is resolved via [`resolve_decision_by_description`]
/// over a graph rebuilt from `handle`. For the tools that write: a close
/// candidate is listed, never picked.
///
/// Both ambiguity and not-found are success, rendered as the identical
/// `{result_count, truncated, latency_ms, data: {outcome: ..., ...}}`
/// envelope the CLI's `--json` output uses for the same outcomes (all three
/// resolutions serialize the same `QueryResponse<ResolveOutcome>`).
pub(crate) fn resolve_target<L: EventLedger>(
    handle: &LedgerHandle<L>,
    id: Option<&str>,
    description: Option<&str>,
    topic: Option<&str>,
    selector_field: &str,
) -> Result<ResolvedTarget, CoreError> {
    resolve_target_as(
        handle,
        id,
        description,
        topic,
        selector_field,
        resolve_decision_by_description,
    )
    .map(|(target, _)| target)
}

/// [`resolve_target`] for the tools that only read (`get_decision_neighborhood`,
/// `get_decision_outcome`, `get_supersession_chain`, `get_compact_view`): the description
/// resolves via [`resolve_decision_for_reading`], so a decision that lacks some of its words but
/// leads alone is the target. Alongside the target it returns that close match, which the tool
/// puts in its reply with `read_reply` so the caller sees what the decision lacks.
pub(crate) fn resolve_target_for_reading<L: EventLedger>(
    handle: &LedgerHandle<L>,
    id: Option<&str>,
    description: Option<&str>,
    topic: Option<&str>,
    selector_field: &str,
) -> Result<(ResolvedTarget, Option<ResolvedCandidate>), CoreError> {
    resolve_target_as(
        handle,
        id,
        description,
        topic,
        selector_field,
        resolve_decision_for_reading,
    )
}

/// A read tool's reply: the query response's envelope, labelled with the close match its target
/// was resolved from, when there was one (`close_match` beside `data`).
fn read_reply<T: serde::Serialize>(
    response: &QueryResponse<T>,
    close_match: Option<&ResolvedCandidate>,
) -> ToolOutput {
    let mut envelope = json!({
        "result_count": response.result_count,
        "truncated": response.truncated,
        "latency_ms": response.latency_ms,
        "data": response.data,
    });
    if let Some(candidate) = close_match {
        annotate_resolution(&mut envelope, candidate);
    }
    ToolOutput(envelope)
}

fn resolve_target_as<L: EventLedger>(
    handle: &LedgerHandle<L>,
    id: Option<&str>,
    description: Option<&str>,
    topic: Option<&str>,
    selector_field: &str,
    resolve: fn(&SharedGraph, &str, Option<&str>) -> crate::Result<QueryResponse<ResolveOutcome>>,
) -> Result<(ResolvedTarget, Option<ResolvedCandidate>), CoreError> {
    if let Some(id) = id {
        return Ok((ResolvedTarget::Id(id.to_owned()), None));
    }

    let Some(description) = description.map(str::trim).filter(|value| !value.is_empty()) else {
        return Err(CoreError::InvalidArgument(missing_selector_message(
            selector_field,
        )));
    };

    let graph = handle.graph().map_err(CoreError::from)?;
    let response = resolve(&graph, description, topic).map_err(CoreError::from)?;

    let QueryResponse {
        result_count,
        truncated,
        latency_ms,
        data,
    } = response;
    match data {
        ResolveOutcome::Resolved { candidate } => {
            let close_match = candidate.needs_annotation().then(|| candidate.clone());
            Ok((ResolvedTarget::Id(candidate.decision_id), close_match))
        }
        ResolveOutcome::NotFound => Ok((
            ResolvedTarget::NotFound(ToolOutput(json!({
                "result_count": result_count,
                "truncated": truncated,
                "latency_ms": latency_ms,
                "data": ResolveOutcome::NotFound,
            }))),
            None,
        )),
        ResolveOutcome::Ambiguous { candidates } => {
            let data = ResolveOutcome::Ambiguous { candidates };
            Ok((
                ResolvedTarget::Ambiguous(ToolOutput(json!({
                    "result_count": result_count,
                    "truncated": truncated,
                    "latency_ms": latency_ms,
                    "data": data,
                }))),
                None,
            ))
        }
    }
}

// ---------------------------------------------------------------------------
// get_decision_neighborhood (the CLI's `why`)
// ---------------------------------------------------------------------------

/// Parsed, validated arguments for the `get_decision_neighborhood` tool.
pub(crate) struct GetDecisionNeighborhoodArgs {
    pub(crate) decision_id: Option<String>,
    pub(crate) description: Option<String>,
    pub(crate) topic: Option<String>,
}

impl GetDecisionNeighborhoodArgs {
    pub(crate) fn from_json(args: &Map<String, Value>) -> Result<Self, CoreError> {
        Ok(Self {
            decision_id: optional_string(args, "decision_id")?,
            description: optional_string(args, "description")?,
            topic: optional_string(args, "topic")?,
        })
    }
}

/// The migrated core for the `get_situational_decisions` MCP tool: one
/// implementation consumed by both transports.
///
/// `graph` is supplied by the caller rather than resolved via
/// [`LedgerProvider`]: stdio rebuilds a fresh [`crate::projector::memory::MemoryGraph`]
/// per call, while HTTP reuses its per-tenant graph cache. That is a caching
/// decision, not a tenancy one, so it stays outside the provider seam —
/// [`LedgerProvider`] here is only used for the ledger the query needs to
/// resolve `since_offset`/`since_timestamp` change annotations.
pub(crate) fn get_situational_decisions<P: LedgerProvider>(
    provider: &P,
    graph: &impl GraphView,
    args: GetSituationalDecisionsArgs,
) -> Result<ToolOutput, CoreError> {
    let handle = provider.ledger()?;
    let context = QueryContext::new(handle.tenant_id);
    let request = SituationalRequest {
        paths: args.paths,
        since_offset: args.since_offset,
        since_timestamp: args.since_timestamp,
        limit: args.limit,
        cursor: args.cursor,
        project: args.project,
    };
    let response =
        crate::queries::get_situational_decisions(&context, graph, &handle.ledger, &request)
            .map_err(CoreError::from)?;
    Ok(ToolOutput(json!({
        "result_count": response.result_count,
        "truncated": response.truncated,
        "latency_ms": response.latency_ms,
        "data": response.data,
    })))
}

/// The migrated core for the `get_decision_neighborhood` MCP tool (the CLI's
/// `why`): one implementation consumed by both transports. Payload is
/// identical to `loomtracer query why --json` — the same `QueryResponse`
/// envelope around [`crate::queries::NeighborhoodView`] with the decision's
/// quality profile on the root (`data.root.quality`), always at the
/// CLI default (depth 1, all relations, non-compact); this bead does not
/// expose those filters over MCP.
pub(crate) fn get_decision_neighborhood<P: LedgerProvider>(
    provider: &P,
    args: GetDecisionNeighborhoodArgs,
) -> Result<ToolOutput, CoreError> {
    let handle = provider.ledger()?;
    let (target, close_match) = resolve_target_for_reading(
        &handle,
        args.decision_id.as_deref(),
        args.description.as_deref(),
        args.topic.as_deref(),
        "decision_id",
    )?;
    let decision_id = match target {
        ResolvedTarget::Id(id) => id,
        ResolvedTarget::Ambiguous(output) => return Ok(output),
        ResolvedTarget::NotFound(output) => return Ok(output),
    };

    let graph = handle.graph().map_err(CoreError::from)?;
    let mut response =
        query_get_decision_neighborhood(&graph, &decision_id, &NeighborhoodRequest::all())
            .map_err(CoreError::from)?;
    // The graph holds no per-edge times: the ledger supplies the dated story (hivemind-bbnw.7).
    response.data.timeline = query_get_decision_timeline(&graph, &handle.ledger, &decision_id)
        .map_err(CoreError::from)?
        .data;
    // The answer to "why" carries how well the record supports the decision (hivemind-bbnw.2).
    let response = quality_profile::why_with_quality(&graph, response).map_err(CoreError::from)?;

    Ok(read_reply(&response, close_match.as_ref()))
}

// ---------------------------------------------------------------------------
// get_waiting_requests, get_contested_decisions, get_changed_decisions (hivemind-bbnw.7)
// ---------------------------------------------------------------------------

/// Days back from now that `get_changed_decisions` covers when it is given no `since`.
const CHANGED_DEFAULT_WINDOW_DAYS: i64 = 7;

/// Parsed, validated arguments for the `get_waiting_requests` tool.
pub(crate) struct GetWaitingRequestsArgs {
    pub(crate) request: WaitingRequestsRequest,
}

impl GetWaitingRequestsArgs {
    pub(crate) fn from_json(args: &Map<String, Value>) -> Result<Self, CoreError> {
        Ok(Self {
            request: WaitingRequestsRequest {
                limit: optional_usize(args, "limit")?.unwrap_or(25),
                cursor: optional_string(args, "cursor")?,
            },
        })
    }
}

/// The core for the `get_waiting_requests` MCP tool (the CLI's `query get_waiting_requests`):
/// open requests with no answering decision yet, oldest first, one page at a time.
pub(crate) fn get_waiting_requests(
    graph: &impl GraphView,
    args: GetWaitingRequestsArgs,
) -> Result<ToolOutput, CoreError> {
    let response = query_get_waiting_requests(graph, &args.request).map_err(CoreError::from)?;
    query_output(&response)
}

/// Parsed, validated arguments for the `get_contested_decisions` tool.
pub(crate) struct GetContestedDecisionsArgs {
    pub(crate) request: ContestedDecisionsRequest,
}

impl GetContestedDecisionsArgs {
    pub(crate) fn from_json(args: &Map<String, Value>) -> Result<Self, CoreError> {
        Ok(Self {
            request: ContestedDecisionsRequest {
                limit: optional_usize(args, "limit")?.unwrap_or(25),
                cursor: optional_string(args, "cursor")?,
            },
        })
    }
}

/// The core for the `get_contested_decisions` MCP tool (the CLI's `query
/// get_contested_decisions`): decisions in contest, oldest first, one page at a time.
pub(crate) fn get_contested_decisions(
    graph: &impl GraphView,
    args: GetContestedDecisionsArgs,
) -> Result<ToolOutput, CoreError> {
    let response = query_get_contested_decisions(graph, &args.request).map_err(CoreError::from)?;
    query_output(&response)
}

/// Parsed, validated arguments for the `get_open_why_decisions` tool.
pub(crate) struct GetOpenWhyDecisionsArgs {
    pub(crate) request: OpenWhyRequest,
}

impl GetOpenWhyDecisionsArgs {
    pub(crate) fn from_json(args: &Map<String, Value>) -> Result<Self, CoreError> {
        Ok(Self {
            request: OpenWhyRequest {
                limit: optional_usize(args, "limit")?.unwrap_or(25),
                cursor: optional_string(args, "cursor")?,
            },
        })
    }
}

/// The core for the `get_open_why_decisions` MCP tool (the CLI's `query
/// get_open_why_decisions`): binding decisions whose why is still to answer, oldest first, one
/// page at a time (hivemind-lapcm).
pub(crate) fn get_open_why_decisions(
    graph: &impl GraphView,
    args: GetOpenWhyDecisionsArgs,
) -> Result<ToolOutput, CoreError> {
    let response = query_get_open_why_decisions(graph, &args.request).map_err(CoreError::from)?;
    query_output(&response)
}

/// Parsed, validated arguments for the `get_changed_decisions` tool.
pub(crate) struct GetChangedDecisionsArgs {
    /// The start of the window; the last week when omitted.
    pub(crate) since: Option<DateTime<Utc>>,
    pub(crate) until: Option<DateTime<Utc>>,
    pub(crate) limit: usize,
    pub(crate) cursor: Option<String>,
}

impl GetChangedDecisionsArgs {
    pub(crate) fn from_json(args: &Map<String, Value>) -> Result<Self, CoreError> {
        Ok(Self {
            since: optional_datetime(args, "since")?,
            until: optional_datetime(args, "until")?,
            limit: optional_usize(args, "limit")?.unwrap_or(25),
            cursor: optional_string(args, "cursor")?,
        })
    }
}

/// The core for the `get_changed_decisions` MCP tool (the CLI's `query get_changed_decisions`):
/// decisions revised, superseded or left without a premise in the window, most recently changed
/// first. It reads the ledger for the dated events and the graph for the decisions, so it takes
/// the provider and a graph. The window it used is echoed in `data.since` / `data.until`.
pub(crate) fn get_changed_decisions<P: LedgerProvider>(
    provider: &P,
    graph: &impl GraphView,
    args: GetChangedDecisionsArgs,
) -> Result<ToolOutput, CoreError> {
    let handle = provider.ledger()?;
    let since = args
        .since
        .unwrap_or_else(|| Utc::now() - Duration::days(CHANGED_DEFAULT_WINDOW_DAYS));
    let request = ChangedDecisionsRequest {
        since: Some(since),
        until: args.until,
        limit: args.limit,
        cursor: args.cursor,
    };
    let response =
        query_get_changed_decisions(graph, &handle.ledger, &request).map_err(CoreError::from)?;
    query_output(&response)
}

// ---------------------------------------------------------------------------
// loomtracer_compact_view (the CLI's `compact-view`)
// ---------------------------------------------------------------------------

/// Parsed, validated arguments for the `loomtracer_compact_view` tool.
pub(crate) struct CompactViewArgs {
    pub(crate) decision_id: Option<String>,
    pub(crate) description: Option<String>,
    pub(crate) topic: Option<String>,
}

impl CompactViewArgs {
    pub(crate) fn from_json(args: &Map<String, Value>) -> Result<Self, CoreError> {
        Ok(Self {
            decision_id: optional_string(args, "decision_id")?,
            description: optional_string(args, "description")?,
            topic: optional_string(args, "topic")?,
        })
    }
}

/// The migrated core for the `loomtracer_compact_view` MCP tool: one
/// implementation consumed by both transports. Payload is identical to
/// `loomtracer query compact-view --json` — the same `QueryResponse` envelope
/// around `Option<CompactView>` from [`crate::queries::get_compact_view`].
/// `data: null` there means the resolved `decision_id` itself doesn't exist
/// (the id escape hatch bypasses resolution, so an unvalidated id can still
/// miss); that's distinct from the resolver's own not-found outcome, which
/// fires when a free-text `description` matches nothing and is signalled by
/// `data.outcome` instead — same split [`get_decision_neighborhood`] already
/// established.
pub(crate) fn compact_view<P: LedgerProvider>(
    provider: &P,
    args: CompactViewArgs,
) -> Result<ToolOutput, CoreError> {
    let handle = provider.ledger()?;
    let (target, close_match) = resolve_target_for_reading(
        &handle,
        args.decision_id.as_deref(),
        args.description.as_deref(),
        args.topic.as_deref(),
        "decision_id",
    )?;
    let decision_id = match target {
        ResolvedTarget::Id(id) => id,
        ResolvedTarget::Ambiguous(output) => return Ok(output),
        ResolvedTarget::NotFound(output) => return Ok(output),
    };

    let graph = handle.graph().map_err(CoreError::from)?;
    let response = get_compact_view(&graph, &decision_id).map_err(CoreError::from)?;

    Ok(read_reply(&response, close_match.as_ref()))
}

// ---------------------------------------------------------------------------
// recall_decisions
// ---------------------------------------------------------------------------

fn parse_decision_status(value: &str) -> Result<DecisionStatus, CoreError> {
    match value {
        "proposed" => Ok(DecisionStatus::Proposed),
        "accepted" => Ok(DecisionStatus::Accepted),
        "rejected" => Ok(DecisionStatus::Rejected),
        "contested" => Ok(DecisionStatus::Contested),
        "superseded" => Ok(DecisionStatus::Superseded),
        other => Err(CoreError::InvalidArgument(format!(
            "unknown status `{other}`"
        ))),
    }
}

/// Parsed, validated arguments for the `recall_decisions` tool. No resolver
/// involved: recall takes a free-text search query and filters, not an id.
pub(crate) struct RecallDecisionsArgs {
    pub(crate) q: Option<String>,
    pub(crate) topic_keys: Vec<String>,
    pub(crate) statuses: Vec<DecisionStatus>,
    pub(crate) actor_ids: Vec<String>,
    pub(crate) sources: Vec<String>,
    pub(crate) since: Option<chrono::DateTime<chrono::Utc>>,
    pub(crate) until: Option<chrono::DateTime<chrono::Utc>>,
    pub(crate) limit: usize,
    pub(crate) cursor: Option<String>,
    pub(crate) project: Option<String>,
}

impl RecallDecisionsArgs {
    pub(crate) fn from_json(args: &Map<String, Value>) -> Result<Self, CoreError> {
        let statuses = optional_string_array(args, "status")?
            .iter()
            .map(|status| parse_decision_status(status))
            .collect::<Result<Vec<_>, _>>()?;
        let limit = optional_usize(args, "limit")?.unwrap_or(RECALL_DEFAULT_LIMIT);
        if limit > RECALL_MAX_LIMIT {
            return Err(CoreError::InvalidArgument(format!(
                "limit must be at most {RECALL_MAX_LIMIT}"
            )));
        }
        Ok(Self {
            q: optional_string(args, "q")?,
            topic_keys: optional_string_array(args, "topic")?,
            statuses,
            actor_ids: optional_string_array(args, "actor_id")?,
            sources: optional_string_array(args, "source")?,
            since: optional_datetime(args, "since")?,
            until: optional_datetime(args, "until")?,
            limit,
            cursor: optional_string(args, "cursor")?,
            project: optional_string(args, "project")?,
        })
    }
}

// ---------------------------------------------------------------------------
// disagree_decision
// ---------------------------------------------------------------------------

/// What a person said, as the agent that heard them records it (hivemind-efo6x): `person` is the
/// `human:` actor the words are attributed to, `quote` their verbatim words, `said_in` where they
/// said them. The calling agent is recorded as the relay.
pub(crate) struct RelayedWords {
    pub(crate) person: String,
    pub(crate) quote: String,
    pub(crate) said_in: Option<String>,
}

/// What the caller of `disagree_decision` or `reaffirm_decision` says: its own `reason`, or, when
/// `<verb>_by` names a person, that person's words, recorded as theirs with the caller as the
/// agent that relayed them. A person's name without their words is refused, and so are words
/// without a person to attribute them to: a statement is never both the agent's and a person's.
pub(crate) enum Statement {
    Own { reason: String },
    Relayed(RelayedWords),
}

impl Statement {
    /// `person_key` is the argument naming the person (`disagreed_by`, `reaffirmed_by`).
    fn from_json(args: &Map<String, Value>, person_key: &str) -> Result<Self, CoreError> {
        let person = optional_string(args, person_key)?;
        let reason = optional_string(args, "reason")?;
        let quote = optional_string(args, "quote")?;
        let said_in = optional_string(args, "said_in")?;
        let Some(person) = person else {
            if quote.is_some() || said_in.is_some() {
                return Err(CoreError::InvalidArgument(format!(
                    "`quote` and `said_in` go with `{person_key}`: name the person whose words \
                     they are, or leave them out and give your own `reason`"
                )));
            }
            return Ok(Self::Own {
                reason: require_string(args, "reason")?,
            });
        };
        if reason.is_some() {
            return Err(CoreError::InvalidArgument(format!(
                "with `{person_key}` the person's words are the `quote`: leave `reason` out, \
                 which would be your own"
            )));
        }
        let Some(quote) = quote else {
            return Err(CoreError::InvalidArgument(format!(
                "`{person_key}` needs `quote`, the person's verbatim words as they said them: \
                 a person's name without their words is not recorded"
            )));
        };
        Ok(Self::Relayed(RelayedWords {
            person: person.trim().to_owned(),
            quote,
            said_in,
        }))
    }

    /// The words of the reply that name who a relayed statement is the person's, and who relayed it.
    fn insert_relay_into(&self, reply: &mut Value, person_key: &str, relayed_by: &str) {
        let (Self::Relayed(words), Some(reply)) = (self, reply.as_object_mut()) else {
            return;
        };
        reply.insert(person_key.to_owned(), json!(words.person));
        reply.insert("relayed_by".to_owned(), json!(relayed_by));
        if let Some(said_in) = &words.said_in {
            reply.insert("said_in".to_owned(), json!(said_in.trim()));
        }
    }
}

/// Parsed, validated arguments for the `disagree_decision` tool. Shares the
/// id/description/topic resolution shape [`resolve_target`] defines; what the
/// caller says (`reason`, or a person's `quote`) is the one thing unique to a
/// disagreement, and is always required regardless of how the target resolves.
pub(crate) struct DisagreeArgs {
    pub(crate) actor_id: String,
    pub(crate) decision_id: Option<String>,
    pub(crate) description: Option<String>,
    pub(crate) topic: Option<String>,
    pub(crate) statement: Statement,
}

impl DisagreeArgs {
    pub(crate) fn from_json(
        args: &Map<String, Value>,
        actor_id: String,
    ) -> Result<Self, CoreError> {
        let decision_id = optional_string(args, "decision_id")?;
        let description = optional_string(args, "description")?;
        let topic = optional_string(args, "topic")?;
        let statement = Statement::from_json(args, "disagreed_by")?;
        Ok(Self {
            actor_id,
            decision_id,
            description,
            topic,
            statement,
        })
    }
}

/// Parsed, validated arguments for the `dispute_attribution` tool (hivemind-f9xgu): a person's
/// agent recording, in the person's words, that a decision names them as its decider and the call
/// was never theirs. The target resolves the way `disagree_decision`'s does. `actor_id` is the
/// relaying agent, `disputed_by` the person, and `quote` their verbatim words: all three are
/// required for a person, and the person is never inferred from the credential. A decider agent
/// that says the call was never its own sends its `reason` alone (hivemind-53dmq), recorded as
/// its own dispute with its own credential, the way the CLI's `dispute-attribution --reason` is.
pub(crate) struct DisputeAttributionArgs {
    pub(crate) actor_id: String,
    pub(crate) decision_id: Option<String>,
    pub(crate) description: Option<String>,
    pub(crate) topic: Option<String>,
    pub(crate) statement: Statement,
}

impl DisputeAttributionArgs {
    pub(crate) fn from_json(
        args: &Map<String, Value>,
        actor_id: String,
    ) -> Result<Self, CoreError> {
        let person = optional_string(args, "disputed_by")?;
        let reason = optional_string(args, "reason")?;
        let statement = match (person, reason) {
            (Some(person), None) => {
                let person = person.trim().to_owned();
                if person.starts_with("agent:") {
                    return Err(CoreError::InvalidArgument(format!(
                        "{person} is not a person (human:<name>): `disputed_by` is the person \
                         whose words `quote` carries. To say the call was never yours, leave \
                         out `disputed_by` and `quote` and send your own words as `reason`"
                    )));
                }
                Statement::Relayed(RelayedWords {
                    person,
                    quote: require_string(args, "quote")?,
                    said_in: optional_string(args, "said_in")?,
                })
            }
            (None, Some(reason)) => {
                if optional_string(args, "quote")?.is_some()
                    || optional_string(args, "said_in")?.is_some()
                {
                    return Err(CoreError::InvalidArgument(
                        "`quote` and `said_in` go with `disputed_by`: name the person whose \
                         words they are, or leave them out and give your own `reason`"
                            .to_owned(),
                    ));
                }
                Statement::Own { reason }
            }
            (Some(_), Some(_)) => {
                return Err(CoreError::InvalidArgument(
                    "with `disputed_by` the person's words are the `quote`: leave `reason` out, \
                     which would be your own. Send `disputed_by` with `quote` for a person, or \
                     `reason` alone for yourself"
                        .to_owned(),
                ));
            }
            (None, None) => {
                // A `reason` of only whitespace reads as absent above; say it is empty, not that
                // the person is missing (hivemind-3wgmm).
                let reason_given = matches!(args.get("reason"), Some(Value::String(_)));
                return Err(CoreError::InvalidArgument(if reason_given {
                    format!(
                        "`reason` must be a non-empty string: {DISPUTE_ATTRIBUTION_NEEDS_A_PERSON}"
                    )
                } else {
                    format!("missing `disputed_by`: {DISPUTE_ATTRIBUTION_NEEDS_A_PERSON}")
                }));
            }
        };
        Ok(Self {
            actor_id,
            decision_id: optional_string(args, "decision_id")?,
            description: optional_string(args, "description")?,
            topic: optional_string(args, "topic")?,
            statement,
        })
    }
}

/// The core for the `dispute_attribution` MCP tool: one implementation consumed by both
/// transports. Resolves its target via [`resolve_target`] first (an ambiguous or unmatched
/// description is returned as-is and nothing is written), then records one
/// `decision.attribution_disputed`: as `disputed_by`, relayed by `actor_id`
/// (`Commands::relay_attribution_dispute`), or, for a `reason` alone, as `actor_id` itself
/// (`Commands::dispute_attribution`), which must be an agent the decision names as its decider.
/// The decision's status is unchanged: a dispute of who decided is not a rejection, and the
/// reply says so by returning the status as it now stands.
pub(crate) fn dispute_attribution<P: LedgerProvider>(
    provider: &P,
    args: DisputeAttributionArgs,
) -> Result<ToolOutput, CoreError> {
    let handle = provider.ledger()?;
    let target = resolve_target(
        &handle,
        args.decision_id.as_deref(),
        args.description.as_deref(),
        args.topic.as_deref(),
        "decision_id",
    )?;
    let decision_id = match target {
        ResolvedTarget::Id(id) => id,
        ResolvedTarget::Ambiguous(output) => return Ok(output),
        ResolvedTarget::NotFound(output) => return Ok(output),
    };

    let commands = Commands::new_with_context(
        &handle.ledger,
        CommandContext::new(
            handle.tenant_id.clone(),
            EventProvenance::agent(args.actor_id.clone()),
        ),
    );
    let event_id = match &args.statement {
        Statement::Own { reason } => {
            // The tool's caller is an agent: a person's own dispute goes through their agent with
            // their words, so an actor_id naming a person is not taken as the person speaking.
            // Checked here so the refusal speaks in tools (the commands text names CLI forms).
            if !args.actor_id.starts_with("agent:") {
                return Err(CoreError::InvalidArgument(format!(
                    "{} is not an agent: a person's dispute is recorded for them by their agent, \
                     with `disputed_by` and their verbatim words as `quote`",
                    args.actor_id
                )));
            }
            if !commands
                .is_recorded_decider(&decision_id, &args.actor_id)
                .map_err(CoreError::from)?
            {
                return Err(dispute_attribution_not_a_decider(
                    &args.actor_id,
                    &decision_id,
                ));
            }
            commands.dispute_attribution(&args.actor_id, &decision_id, reason)
        }
        Statement::Relayed(words) => commands.relay_attribution_dispute(
            &args.actor_id,
            &words.person,
            &decision_id,
            &words.quote,
            words.said_in.as_deref(),
        ),
    }
    .map_err(CoreError::from)?;

    let graph = handle.graph().map_err(CoreError::from)?;
    let decision_status = derive_decision_status(&graph, &decision_id).map_err(CoreError::from)?;

    let mut reply = json!({
        "decision_id": decision_id,
        "event_id": event_id,
        "decision_status": decision_status,
    });
    match &args.statement {
        Statement::Own { .. } => {
            if let Some(reply) = reply.as_object_mut() {
                reply.insert("disputed_by".to_owned(), json!(args.actor_id));
            }
        }
        Statement::Relayed(_) => {
            args.statement
                .insert_relay_into(&mut reply, "disputed_by", &args.actor_id)
        }
    }
    Ok(ToolOutput(reply))
}

/// Parsed, validated arguments for the `accept_decision` tool (hivemind-fsxx9): a person's agent
/// recording, in the person's words, that they accept a decision. The target resolves the way
/// `disagree_decision`'s does. `actor_id` is the relaying agent, `accepted_by` the person,
/// `quote` their verbatim words and `question` what they answered: all four are always required,
/// and the person is never inferred from the credential. `said_in` is where they said it, when the
/// agent knows.
pub(crate) struct AcceptDecisionArgs {
    pub(crate) actor_id: String,
    pub(crate) decision_id: Option<String>,
    pub(crate) description: Option<String>,
    pub(crate) topic: Option<String>,
    pub(crate) accepted_by: String,
    pub(crate) quote: String,
    pub(crate) question: String,
    pub(crate) said_in: Option<String>,
}

impl AcceptDecisionArgs {
    pub(crate) fn from_json(
        args: &Map<String, Value>,
        actor_id: String,
    ) -> Result<Self, CoreError> {
        Ok(Self {
            actor_id,
            decision_id: optional_string(args, "decision_id")?,
            description: optional_string(args, "description")?,
            topic: optional_string(args, "topic")?,
            accepted_by: require_string(args, "accepted_by")?.trim().to_owned(),
            quote: require_string(args, "quote")?,
            question: require_string(args, "question")?,
            said_in: optional_string(args, "said_in")?,
        })
    }
}

/// The core for the `accept_decision` MCP tool: one implementation consumed by both transports.
/// Resolves its target via [`resolve_target`] first (an ambiguous or unmatched description is
/// returned as-is and nothing is written), then records one `decision.accepted` as
/// `accepted_by`, relayed by `actor_id` (`Commands::relay_acceptance`). The reply returns the
/// decision's status as it now stands: a person's acceptance of an open proposal makes it
/// `accepted`, and one beside another actor's rejection leaves it `contested`.
pub(crate) fn accept_decision<P: LedgerProvider>(
    provider: &P,
    args: AcceptDecisionArgs,
) -> Result<ToolOutput, CoreError> {
    let handle = provider.ledger()?;
    let target = resolve_target(
        &handle,
        args.decision_id.as_deref(),
        args.description.as_deref(),
        args.topic.as_deref(),
        "decision_id",
    )?;
    let decision_id = match target {
        ResolvedTarget::Id(id) => id,
        ResolvedTarget::Ambiguous(output) => return Ok(output),
        ResolvedTarget::NotFound(output) => return Ok(output),
    };

    let commands = Commands::new_with_context(
        &handle.ledger,
        CommandContext::new(
            handle.tenant_id.clone(),
            EventProvenance::agent(args.actor_id.clone()),
        ),
    );
    let event_id = commands
        .relay_acceptance(
            &args.actor_id,
            args.said_in.as_deref(),
            AcceptanceInput {
                accepted_by: &args.accepted_by,
                decision_id: &decision_id,
                quote: &args.quote,
                question: &args.question,
            },
        )
        .map_err(CoreError::from)?;

    let graph = handle.graph().map_err(CoreError::from)?;
    let decision_status = derive_decision_status(&graph, &decision_id).map_err(CoreError::from)?;

    let mut reply = json!({
        "decision_id": decision_id,
        "event_id": event_id,
        "accepted_by": args.accepted_by,
        "relayed_by": args.actor_id,
        "decision_status": decision_status,
    });
    if let (Some(said_in), Some(reply)) = (args.said_in.as_deref(), reply.as_object_mut()) {
        reply.insert("said_in".to_owned(), json!(said_in.trim()));
    }
    Ok(ToolOutput(reply))
}

/// The migrated core for the `recall_decisions` MCP tool: one implementation
/// consumed by both transports.
///
/// `graph` is supplied by the caller, same caching split as
/// [`get_situational_decisions`]. Unlike that tool, the ledger here must be
/// [`AnyLedger`] specifically rather than any `P::Ledger: EventLedger`:
/// `summarize::recall_decisions` takes the concrete enum, not just the trait.
pub(crate) fn recall_decisions<P>(
    provider: &P,
    graph: &impl GraphView,
    args: RecallDecisionsArgs,
) -> Result<ToolOutput, CoreError>
where
    P: LedgerProvider<Ledger = AnyLedger>,
{
    let handle = provider.ledger()?;
    let context = QueryContext::new(handle.tenant_id);
    let request = RecallRequest {
        q: args.q,
        topic_keys: args.topic_keys,
        statuses: args.statuses,
        actor_ids: args.actor_ids,
        sources: args.sources,
        since: args.since,
        until: args.until,
        limit: args.limit,
        cursor: args.cursor,
        project: args.project,
    };
    let response = crate::summarize::recall_decisions(&context, &handle.ledger, graph, &request)
        .map_err(CoreError::from)?;
    Ok(ToolOutput(json!({
        "result_count": response.result_count,
        "truncated": response.truncated,
        "latency_ms": response.latency_ms,
        "data": response.data,
    })))
}

// ---------------------------------------------------------------------------
// get_decision_outcome (the CLI's `verify`)
// ---------------------------------------------------------------------------

/// Parsed, validated arguments for the `get_decision_outcome` tool.
pub(crate) struct GetDecisionOutcomeArgs {
    pub(crate) decision_id: Option<String>,
    pub(crate) description: Option<String>,
    pub(crate) topic: Option<String>,
}

impl GetDecisionOutcomeArgs {
    pub(crate) fn from_json(args: &Map<String, Value>) -> Result<Self, CoreError> {
        Ok(Self {
            decision_id: optional_string(args, "decision_id")?,
            description: optional_string(args, "description")?,
            topic: optional_string(args, "topic")?,
        })
    }
}

/// The migrated core for the `get_decision_outcome` MCP tool (the CLI's
/// `verify`): one implementation consumed by both transports. Payload is
/// `DecisionBrief` plus the decision's quality profile as `data.quality` — the
/// same envelope `loomtracer query verify --json` (`get_decision_brief`,
/// composing `get_decision` + `get_decision_context` + `get_decision_outcome`)
/// returns, not the narrower outcome-signal-only payload this tool returned
/// before this migration — see hivemind-ot72.8 ("match the CLI").
pub(crate) fn get_decision_outcome<P: LedgerProvider>(
    provider: &P,
    args: GetDecisionOutcomeArgs,
) -> Result<ToolOutput, CoreError> {
    let handle = provider.ledger()?;
    let (target, close_match) = resolve_target_for_reading(
        &handle,
        args.decision_id.as_deref(),
        args.description.as_deref(),
        args.topic.as_deref(),
        "decision_id",
    )?;
    let decision_id = match target {
        ResolvedTarget::Id(id) => id,
        ResolvedTarget::Ambiguous(output) => return Ok(output),
        ResolvedTarget::NotFound(output) => return Ok(output),
    };

    let graph = handle.graph().map_err(CoreError::from)?;
    let response = query_get_decision_brief(&graph, &decision_id).map_err(CoreError::from)?;
    // The answer carries how well the record supports the decision (hivemind-bbnw.2).
    let response =
        quality_profile::verify_with_quality(&graph, response).map_err(CoreError::from)?;

    Ok(read_reply(&response, close_match.as_ref()))
}

// ---------------------------------------------------------------------------
// supersede_decision
// ---------------------------------------------------------------------------

/// Parsed, validated arguments for the `supersede_decision` tool. The old
/// decision is selected the same way `get_decision_neighborhood` selects its
/// target: `old_decision_id` bypasses resolution, otherwise `description`
/// (+ optional `topic`) resolves via [`resolve_target`].
pub(crate) struct SupersedeDecisionArgs {
    pub(crate) actor_id: String,
    pub(crate) old_decision_id: Option<String>,
    pub(crate) description: Option<String>,
    pub(crate) topic: Option<String>,
    pub(crate) title: String,
    pub(crate) rationale: String,
    pub(crate) topic_keys: Vec<String>,
    pub(crate) option_labels: Vec<String>,
    pub(crate) chosen_option_label: Option<String>,
    /// Keep the replacement at `proposed` even though `chosen_option_label` is set. See
    /// `still_proposed` on `SupersedeInput`.
    pub(crate) still_proposed: bool,
    /// What the replacement rests on; see [`CaptureDecisionArgs::grounding`].
    pub(crate) grounding: GroundingSpec,
    pub(crate) expressed_confidence: Option<String>,
    /// Registered project handle to file the superseding decision under; omitted means it
    /// inherits the old decision's project. See [`project_args`].
    pub(crate) project: Option<String>,
    /// How `project` was determined; `stated` when omitted. Requires `project`.
    pub(crate) project_source: Option<ProjectSource>,
    /// What the replacement binds and what undoing it costs; both required, as for
    /// `capture_decision`.
    pub(crate) floor: FloorAnswers,
    /// Topic keys this supersession adds to its project's vocabulary; each must be one of the
    /// keys it ends up with. See `Commands::declaring_topics`.
    pub(crate) declare_topics: Vec<String>,
}

impl SupersedeDecisionArgs {
    pub(crate) fn from_json(
        args: &Map<String, Value>,
        actor_id: String,
    ) -> Result<Self, CoreError> {
        let (project, project_source) = project_args(args)?;
        Ok(Self {
            actor_id,
            project,
            project_source,
            old_decision_id: optional_string(args, "old_decision_id")?,
            description: optional_string(args, "description")?,
            topic: optional_string(args, "topic")?,
            title: require_string(args, "title")?,
            rationale: require_string(args, "rationale")?,
            topic_keys: topic_keys_arg(args, "topic_keys", false)?,
            option_labels: optional_option_labels(args, "options")?,
            chosen_option_label: optional_string(args, "chosen_option_label")?,
            still_proposed: optional_bool(args, "still_proposed")?,
            grounding: require_wire_grounding(args, WIRE_GROUNDING_REFUSAL)?,
            floor: wire_floor(args)?,
            expressed_confidence: optional_string(args, "expressed_confidence")?,
            declare_topics: optional_string_array(args, "declare_topics")?,
        })
    }
}

/// The migrated core for the `supersede_decision` MCP tool: one
/// implementation consumed by both transports. The old decision is resolved
/// exactly like [`get_decision_neighborhood`]'s target via [`resolve_target`];
/// an ambiguous or not-found resolution is returned as-is, with
/// `commands.supersede` never called — no ledger write happens for either
/// outcome, matching Alex's 2026-09-20 fluent-resolution call for the rest of
/// this tool surface.
pub(crate) fn supersede_decision<P: LedgerProvider>(
    provider: &P,
    args: SupersedeDecisionArgs,
) -> Result<ToolOutput, CoreError> {
    let handle = provider.ledger()?;
    let target = resolve_target(
        &handle,
        args.old_decision_id.as_deref(),
        args.description.as_deref(),
        args.topic.as_deref(),
        "old_decision_id",
    )?;
    let old_decision_id = match target {
        ResolvedTarget::Id(id) => id,
        ResolvedTarget::Ambiguous(output) => return Ok(output),
        ResolvedTarget::NotFound(output) => return Ok(output),
    };

    let resolved =
        match resolve_grounding_in(args.grounding, || handle.graph()).map_err(CoreError::from)? {
            GroundingResolution::Ready(resolved) => resolved,
            GroundingResolution::Unresolved(unresolved) => {
                return Ok(ToolOutput(unresolved.envelope()));
            }
        };

    let commands = Commands::new_with_context(
        &handle.ledger,
        CommandContext::new(
            handle.tenant_id.clone(),
            EventProvenance::agent(args.actor_id.clone()),
        ),
    )
    .declaring_topics(&args.declare_topics);
    let outcome = commands
        .supersede(SupersedeInput {
            project: determined_project(args.project.as_deref(), args.project_source),
            actor_id: &args.actor_id,
            old_decision_id: &old_decision_id,
            new_title: &args.title,
            new_rationale: &args.rationale,
            topic_keys: &args.topic_keys,
            option_labels: &args.option_labels,
            chosen_option_label: args.chosen_option_label.as_deref(),
            still_proposed: args.still_proposed,
            // The plan carries every id; see `SupersedeInput::grounding`.
            hypothesis_ids: &[],
            evidence_ids: &[],
            grounding: Some(&resolved.plan),
            expressed_confidence: args.expressed_confidence.as_deref(),
            floor: FloorInput::Stated(&args.floor),
        })
        .map_err(CoreError::from)?;

    let graph = handle.graph().map_err(CoreError::from)?;
    let old_decision_status =
        derive_decision_status(&graph, &old_decision_id).map_err(CoreError::from)?;
    let new_decision_status =
        derive_decision_status(&graph, &outcome.new_decision_id).map_err(CoreError::from)?;
    let upstream = upstream_of(&graph, &old_decision_id, Some(&outcome.new_decision_id))
        .map_err(CoreError::from)?;

    let mut reply = json!({
        "old_decision_id": old_decision_id,
        "new_decision_id": outcome.new_decision_id,
        "proposal_event_id": outcome.proposal_event_id,
        "relation_event_ids": outcome.relation_event_ids,
        "superseded_event_id": outcome.superseded_event_id,
        "old_decision_status": old_decision_status,
        "new_decision_status": new_decision_status,
        "rests_on": resolved.label(outcome.rests_on),
        "premise_stale": outcome.premise_stale,
        // `rests_on` above is what the replacement rests on; this is what the decision it
        // replaced rested on that still stands, which the supersession may really be about.
        "raise_upstream": upstream.raise_upstream,
    });
    args.floor.reply().insert_into(&mut reply);
    insert_placement(&mut reply, &outcome.placement);
    insert_warnings_into(&outcome.warnings, &mut reply);
    Ok(ToolOutput(reply))
}

/// The migrated core for the `disagree_decision` MCP tool: one implementation
/// consumed by both transports. Resolves its target via [`resolve_target`]
/// first; on `Ambiguous` or `NotFound` the resolver's envelope is returned
/// as-is and no event is appended — "never act on a guess" holds for a write
/// tool exactly as it does for the read-only `get_decision_neighborhood`.
pub(crate) fn disagree_decision<P: LedgerProvider>(
    provider: &P,
    args: DisagreeArgs,
) -> Result<ToolOutput, CoreError> {
    let handle = provider.ledger()?;
    let target = resolve_target(
        &handle,
        args.decision_id.as_deref(),
        args.description.as_deref(),
        args.topic.as_deref(),
        "decision_id",
    )?;
    let decision_id = match target {
        ResolvedTarget::Id(id) => id,
        ResolvedTarget::Ambiguous(output) => return Ok(output),
        ResolvedTarget::NotFound(output) => return Ok(output),
    };

    let commands = Commands::new_with_context(
        &handle.ledger,
        CommandContext::new(
            handle.tenant_id.clone(),
            EventProvenance::agent(args.actor_id.clone()),
        ),
    );
    let event_id = match &args.statement {
        Statement::Own { reason } => {
            commands.disagree_by_tool(&args.actor_id, &decision_id, reason)
        }
        Statement::Relayed(words) => commands.relay_disagreement(
            &args.actor_id,
            &words.person,
            &decision_id,
            &words.quote,
            words.said_in.as_deref(),
        ),
    }
    .map_err(CoreError::from)?;

    let graph = handle.graph().map_err(CoreError::from)?;
    let decision_status = derive_decision_status(&graph, &decision_id).map_err(CoreError::from)?;
    let upstream = upstream_of(&graph, &decision_id, None).map_err(CoreError::from)?;

    let mut reply = json!({
        "decision_id": decision_id,
        "event_id": event_id,
        "decision_status": decision_status,
        "rests_on": upstream.rests_on,
        "raise_upstream": upstream.raise_upstream,
    });
    args.statement
        .insert_relay_into(&mut reply, "disagreed_by", &args.actor_id);
    Ok(ToolOutput(reply))
}

// ---------------------------------------------------------------------------
// move_decision
// ---------------------------------------------------------------------------

/// Parsed, validated arguments for the `move_decision` tool. The decision is selected like
/// `disagree_decision`'s: `decision_id` bypasses resolution, otherwise `description` (+ optional
/// `topic`) resolves via [`resolve_target`]. `to` is required whichever way the target resolves;
/// where the decision is now is read from the ledger, never passed.
pub(crate) struct MoveDecisionArgs {
    pub(crate) actor_id: String,
    pub(crate) decision_id: Option<String>,
    pub(crate) description: Option<String>,
    pub(crate) topic: Option<String>,
    pub(crate) to: String,
    pub(crate) reason: Option<String>,
}

impl MoveDecisionArgs {
    pub(crate) fn from_json(
        args: &Map<String, Value>,
        actor_id: String,
    ) -> Result<Self, CoreError> {
        Ok(Self {
            actor_id,
            decision_id: optional_string(args, "decision_id")?,
            description: optional_string(args, "description")?,
            topic: optional_string(args, "topic")?,
            to: require_string(args, "to")?.trim().to_owned(),
            reason: optional_string(args, "reason")?,
        })
    }
}

/// The migrated core for the `move_decision` MCP tool: one implementation consumed by both
/// transports. Resolves its target via [`resolve_target`] first; on `Ambiguous` or `NotFound`
/// the resolver's envelope is returned as-is and no event is appended — the same write gate
/// `disagree_decision` and `supersede_decision` apply, so a move never acts on a guess. The
/// reply is [`crate::commands::DecisionMoveOutcome`], the shape `loomtracer move --json` prints.
pub(crate) fn move_decision<P: LedgerProvider>(
    provider: &P,
    args: MoveDecisionArgs,
) -> Result<ToolOutput, CoreError> {
    let handle = provider.ledger()?;
    let target = resolve_target(
        &handle,
        args.decision_id.as_deref(),
        args.description.as_deref(),
        args.topic.as_deref(),
        "decision_id",
    )?;
    let decision_id = match target {
        ResolvedTarget::Id(id) => id,
        ResolvedTarget::Ambiguous(output) => return Ok(output),
        ResolvedTarget::NotFound(output) => return Ok(output),
    };

    let commands = Commands::new_with_context(
        &handle.ledger,
        CommandContext::new(
            handle.tenant_id.clone(),
            EventProvenance::agent(args.actor_id.clone()),
        ),
    );
    let outcome = commands
        .move_decision_to(
            &args.actor_id,
            &decision_id,
            &args.to,
            args.reason.as_deref(),
        )
        .map_err(CoreError::from)?;

    serde_json::to_value(outcome)
        .map(ToolOutput)
        .map_err(|error| CoreError::Internal(error.to_string()))
}

// ---------------------------------------------------------------------------
// retitle_decision
// ---------------------------------------------------------------------------

/// Parsed, validated arguments for the `retitle_decision` tool. The decision is selected like
/// `move_decision`'s: `decision_id` bypasses resolution, otherwise `description` (+ optional
/// `topic`) resolves via [`resolve_target`]. `to` is required whichever way the target resolves;
/// the decision's current title is read from the ledger, never passed.
pub(crate) struct RetitleDecisionArgs {
    pub(crate) actor_id: String,
    pub(crate) decision_id: Option<String>,
    pub(crate) description: Option<String>,
    pub(crate) topic: Option<String>,
    pub(crate) to: String,
    pub(crate) reason: Option<String>,
}

impl RetitleDecisionArgs {
    pub(crate) fn from_json(
        args: &Map<String, Value>,
        actor_id: String,
    ) -> Result<Self, CoreError> {
        Ok(Self {
            actor_id,
            decision_id: optional_string(args, "decision_id")?,
            description: optional_string(args, "description")?,
            topic: optional_string(args, "topic")?,
            to: require_string(args, "to")?.trim().to_owned(),
            reason: optional_string(args, "reason")?,
        })
    }
}

/// The migrated core for the `retitle_decision` MCP tool: one implementation consumed by both
/// transports. Resolves its target via [`resolve_target`] first; on `Ambiguous` or `NotFound`
/// the resolver's envelope is returned as-is and no event is appended — the same write gate
/// `move_decision` and `disagree_decision` apply, so a retitle never acts on a guess. The reply
/// is [`crate::commands::DecisionRetitleOutcome`], the shape `loomtracer retitle --json` prints.
pub(crate) fn retitle_decision<P: LedgerProvider>(
    provider: &P,
    args: RetitleDecisionArgs,
) -> Result<ToolOutput, CoreError> {
    let handle = provider.ledger()?;
    let target = resolve_target(
        &handle,
        args.decision_id.as_deref(),
        args.description.as_deref(),
        args.topic.as_deref(),
        "decision_id",
    )?;
    let decision_id = match target {
        ResolvedTarget::Id(id) => id,
        ResolvedTarget::Ambiguous(output) => return Ok(output),
        ResolvedTarget::NotFound(output) => return Ok(output),
    };

    let commands = Commands::new_with_context(
        &handle.ledger,
        CommandContext::new(
            handle.tenant_id.clone(),
            EventProvenance::agent(args.actor_id.clone()),
        ),
    );
    let outcome = commands
        .retitle_decision_to(
            &args.actor_id,
            &decision_id,
            &args.to,
            args.reason.as_deref(),
        )
        .map_err(CoreError::from)?;

    serde_json::to_value(outcome)
        .map(ToolOutput)
        .map_err(|error| CoreError::Internal(error.to_string()))
}

// ---------------------------------------------------------------------------
// link_supersession (hivemind-4p0xd)
// ---------------------------------------------------------------------------

/// Parsed, validated arguments for the `link_supersession` tool: two decision ids, both already
/// recorded. They are named `old_decision_id` and `new_decision_id`, not `decision_id`, because
/// with two ids a bare `decision_id` would not say which way the link runs and a caller could
/// record it backwards. `supersede_decision` names its old decision `old_decision_id` too.
pub(crate) struct LinkSupersessionArgs {
    pub(crate) actor_id: String,
    pub(crate) old_decision_id: String,
    pub(crate) new_decision_id: String,
}

impl LinkSupersessionArgs {
    pub(crate) fn from_json(
        args: &Map<String, Value>,
        actor_id: String,
    ) -> Result<Self, CoreError> {
        Ok(Self {
            actor_id,
            old_decision_id: require_string(args, "old_decision_id")?.trim().to_owned(),
            new_decision_id: require_string(args, "new_decision_id")?.trim().to_owned(),
        })
    }
}

/// The migrated core for the `link_supersession` MCP tool: one implementation consumed by both
/// transports. Records `decision.superseded` for two decisions that are already recorded,
/// attributed to `actor_id`, and creates nothing: the new decision keeps its own decider. An id
/// that is not a recorded decision, the same id twice, and a link that would close a loop of
/// replacements are refused as invalid arguments with nothing written; a pair already linked
/// writes nothing and the reply says `linked: false`. The rules live in
/// [`crate::supersession_link`].
pub(crate) fn link_supersession<P: LedgerProvider>(
    provider: &P,
    args: LinkSupersessionArgs,
) -> Result<ToolOutput, CoreError> {
    let handle = provider.ledger()?;
    let graph = handle.graph().map_err(CoreError::from)?;
    let outcome = crate::supersession_link::link_supersession(
        &handle.ledger,
        &handle.tenant_id,
        EventProvenance::agent(args.actor_id.clone()),
        &graph,
        SupersessionLinkCall {
            actor_id: &args.actor_id,
            old_decision_id: &args.old_decision_id,
            new_decision_id: &args.new_decision_id,
        },
    )
    .map_err(CoreError::from)?;
    match outcome {
        SupersessionLinkOutcome::Linked(reply) => serde_json::to_value(reply)
            .map(ToolOutput)
            .map_err(|error| CoreError::Internal(error.to_string())),
        SupersessionLinkOutcome::Refused(text) => Err(CoreError::InvalidArgument(text)),
    }
}

// ---------------------------------------------------------------------------
// acknowledge_suggestion
// ---------------------------------------------------------------------------

/// The channel an acknowledgement made through MCP is recorded as surfaced over when the caller
/// names none. A label the consumer owns; Loomtracer attaches no meaning to it.
pub(crate) const DEFAULT_ACK_CHANNEL: &str = "mcp";

/// Parsed, validated arguments for the `acknowledge_suggestion` tool. `finding_id` and
/// `decision_id` are both read off the finding `get_suggestions` returned.
pub(crate) struct AcknowledgeSuggestionArgs {
    pub(crate) actor_id: String,
    pub(crate) finding_id: String,
    pub(crate) decision_id: String,
    pub(crate) action: AckAction,
    pub(crate) channel: String,
}

impl AcknowledgeSuggestionArgs {
    pub(crate) fn from_json(
        args: &Map<String, Value>,
        actor_id: String,
    ) -> Result<Self, CoreError> {
        let action = match optional_string(args, "action")? {
            None => AckAction::Seen,
            Some(name) => AckAction::parse(name.trim()).ok_or_else(|| {
                let valid: Vec<&str> = AckAction::ALL.iter().map(|a| a.as_str()).collect();
                CoreError::InvalidArgument(format!(
                    "unknown action `{name}`; expected one of {}",
                    valid.join(", ")
                ))
            })?,
        };
        Ok(Self {
            actor_id,
            finding_id: require_string(args, "finding_id")?.trim().to_owned(),
            decision_id: require_string(args, "decision_id")?.trim().to_owned(),
            action,
            channel: optional_string(args, "channel")?
                .map_or_else(|| DEFAULT_ACK_CHANNEL.to_owned(), |c| c.trim().to_owned()),
        })
    }
}

/// The core for the `acknowledge_suggestion` MCP tool, one implementation for both transports.
/// Appends `suggestion.surfaced` and the `notification.acknowledged` that names it through the
/// ordinary actor-attributed write path; the reply is
/// [`crate::commands::SuggestionAcknowledgement`]. No finding is looked up: the write path does
/// not run the scan (layer 3).
pub(crate) fn acknowledge_suggestion<P: LedgerProvider>(
    provider: &P,
    args: AcknowledgeSuggestionArgs,
) -> Result<ToolOutput, CoreError> {
    let handle = provider.ledger()?;
    let commands = Commands::new_with_context(
        &handle.ledger,
        CommandContext::new(
            handle.tenant_id.clone(),
            EventProvenance::agent(args.actor_id.clone()),
        ),
    );
    let outcome = commands
        .acknowledge_suggestion(
            &args.actor_id,
            &args.finding_id,
            &args.decision_id,
            args.action,
            &args.channel,
        )
        .map_err(CoreError::from)?;

    serde_json::to_value(outcome)
        .map(ToolOutput)
        .map_err(|error| CoreError::Internal(error.to_string()))
}

// ---------------------------------------------------------------------------
// get_supersession_chain (the CLI's `chain`)
// ---------------------------------------------------------------------------

/// Parsed, validated arguments for the `get_supersession_chain` tool.
pub(crate) struct GetSupersessionChainArgs {
    pub(crate) decision_id: Option<String>,
    pub(crate) description: Option<String>,
    pub(crate) topic: Option<String>,
}

impl GetSupersessionChainArgs {
    pub(crate) fn from_json(args: &Map<String, Value>) -> Result<Self, CoreError> {
        Ok(Self {
            decision_id: optional_string(args, "decision_id")?,
            description: optional_string(args, "description")?,
            topic: optional_string(args, "topic")?,
        })
    }
}

/// The migrated core for the `get_supersession_chain` MCP tool (the CLI's
/// `chain`): one implementation consumed by both transports. Payload is
/// identical to `loomtracer query chain --json`, resolving `decision_id` or a
/// free-text `description` through the same [`resolve_target`] every other
/// fluent tool uses — ambiguous and not-found are both success envelopes,
/// never errors.
pub(crate) fn get_supersession_chain<P: LedgerProvider>(
    provider: &P,
    args: GetSupersessionChainArgs,
) -> Result<ToolOutput, CoreError> {
    let handle = provider.ledger()?;
    let (target, close_match) = resolve_target_for_reading(
        &handle,
        args.decision_id.as_deref(),
        args.description.as_deref(),
        args.topic.as_deref(),
        "decision_id",
    )?;
    let decision_id = match target {
        ResolvedTarget::Id(id) => id,
        ResolvedTarget::Ambiguous(output) => return Ok(output),
        ResolvedTarget::NotFound(output) => return Ok(output),
    };

    let graph = handle.graph().map_err(CoreError::from)?;
    let response = query_get_supersession_chain(&graph, &decision_id).map_err(CoreError::from)?;

    Ok(read_reply(&response, close_match.as_ref()))
}

// ---------------------------------------------------------------------------
// ground_decision
// ---------------------------------------------------------------------------

/// The refusal for a `ground_decision` call that names nothing the decision rests on.
const WIRE_GROUND_REFUSAL: &str = "nothing to ground: pass `grounding` with at least one item — {kind:\"decision\", description|decision_id} (a decision already made), {kind:\"evidence\", content, source?} (something observed, and where), {kind:\"assumption\", statement} (something assumed), or {kind:\"bet\", statement?, would_change_if?, check_by?} (nothing yet: a declared bet); or pass `answers` (the question this decision answers)";

/// Parsed, validated arguments for the `ground_decision` tool. The decision is selected the way
/// `disagree_decision` selects its target: `decision_id` bypasses resolution, otherwise
/// `description` (+ optional `topic`) resolves via [`resolve_target`].
pub(crate) struct GroundDecisionArgs {
    pub(crate) actor_id: String,
    pub(crate) decision_id: Option<String>,
    pub(crate) description: Option<String>,
    pub(crate) topic: Option<String>,
    /// What the decision rests on; see [`CaptureDecisionArgs::grounding`]. May name nothing when
    /// `answers` is given.
    pub(crate) grounding: GroundingSpec,
    /// The question the decision answers (hivemind-zdsh.16). May stand alone.
    pub(crate) answers: Option<String>,
}

impl GroundDecisionArgs {
    pub(crate) fn from_json(
        args: &Map<String, Value>,
        actor_id: String,
    ) -> Result<Self, CoreError> {
        let answers = optional_string(args, "answers")?;
        let grounding = if answers.is_some() {
            wire_grounding(args)?
        } else {
            require_wire_grounding(args, WIRE_GROUND_REFUSAL)?
        };
        Ok(Self {
            actor_id,
            decision_id: optional_string(args, "decision_id")?,
            description: optional_string(args, "description")?,
            topic: optional_string(args, "topic")?,
            grounding,
            answers,
        })
    }
}

/// The migrated core for the `ground_decision` MCP tool: one implementation consumed by both
/// transports. The decision is resolved via [`resolve_target`] and every named premise via
/// [`resolve_grounding`]; an ambiguous or not-found target or premise is returned as-is with no
/// event appended. A premise that would close a `FOLLOWS_FROM` loop is refused as an invalid
/// argument, also with nothing written. What is recorded is attributed to `actor_id` — the
/// grounder — with no causation link to the decision's proposal.
pub(crate) fn ground_decision<P: LedgerProvider>(
    provider: &P,
    args: GroundDecisionArgs,
) -> Result<ToolOutput, CoreError> {
    let handle = provider.ledger()?;
    let target = resolve_target(
        &handle,
        args.decision_id.as_deref(),
        args.description.as_deref(),
        args.topic.as_deref(),
        "decision_id",
    )?;
    let decision_id = match target {
        ResolvedTarget::Id(id) => id,
        ResolvedTarget::Ambiguous(output) => return Ok(output),
        ResolvedTarget::NotFound(output) => return Ok(output),
    };

    let resolved =
        match resolve_grounding_in(args.grounding, || handle.graph()).map_err(CoreError::from)? {
            GroundingResolution::Ready(resolved) => resolved,
            GroundingResolution::Unresolved(unresolved) => {
                return Ok(ToolOutput(unresolved.envelope()));
            }
        };

    let graph = handle.graph().map_err(CoreError::from)?;
    if let Some(refusal) =
        premise_cycle_refusal(&graph, &decision_id, &resolved.plan).map_err(CoreError::from)?
    {
        return Err(CoreError::InvalidArgument(refusal));
    }

    let commands = Commands::new_with_context(
        &handle.ledger,
        CommandContext::new(
            handle.tenant_id.clone(),
            EventProvenance::agent(args.actor_id.clone()),
        ),
    );
    let added = commands
        .ground_and_answer(
            &args.actor_id,
            &decision_id,
            &resolved.plan,
            args.answers.as_deref(),
        )
        .map_err(CoreError::from)?;

    let mut reply = json!({
        "decision_id": added.decision_id,
        "actor_id": args.actor_id,
        "relation_event_ids": added.relation_event_ids,
        "rests_on": resolved.label(added.rests_on),
        "premise_stale": added.premise_stale,
    });
    if let Some(question) = added.question {
        reply["answers"] = json!({
            "question_id": question.question_id,
            "reused": question.reused,
        });
    }
    Ok(ToolOutput(reply))
}

// ---------------------------------------------------------------------------
// reaffirm_decision / get_reexamine_items (hivemind-ottw.4)
// ---------------------------------------------------------------------------

/// Parsed, validated arguments for the `reaffirm_decision` tool. The decision is selected the way
/// `disagree_decision` selects its target (`decision_id` bypasses resolution, otherwise
/// `description` + optional `topic`); `premise_id` names which changed premise it still holds
/// after and is needed only when the decision has several open items; `reason` is required.
pub(crate) struct ReaffirmDecisionArgs {
    pub(crate) actor_id: String,
    pub(crate) decision_id: Option<String>,
    pub(crate) description: Option<String>,
    pub(crate) topic: Option<String>,
    pub(crate) premise_id: Option<String>,
    pub(crate) statement: Statement,
}

impl ReaffirmDecisionArgs {
    pub(crate) fn from_json(
        args: &Map<String, Value>,
        actor_id: String,
    ) -> Result<Self, CoreError> {
        Ok(Self {
            actor_id,
            decision_id: optional_string(args, "decision_id")?,
            description: optional_string(args, "description")?,
            topic: optional_string(args, "topic")?,
            premise_id: optional_string(args, "premise_id")?,
            statement: Statement::from_json(args, "reaffirmed_by")?,
        })
    }
}

/// The core for the `reaffirm_decision` MCP tool: one implementation consumed by both transports.
/// Says that a decision still holds after something it rests on changed, closing its open
/// re-examine item. The target resolves via [`resolve_target`] (an ambiguous or unmatched
/// description is returned as-is, nothing written), then `crate::reaffirmation::reaffirm` checks
/// that an open item is named: several open items and no `premise_id` is a success-shaped
/// `{outcome: "ambiguous"}` listing them, and anything with nothing to re-examine (the premise did
/// not change, the item was already closed, the decision does not rest on it) is refused as an
/// invalid argument, with nothing written. What is recorded is attributed to `actor_id`, or, when
/// the call carries a person's words (`reaffirmed_by` and `quote`), to that person with `actor_id`
/// recorded as the agent that relayed them.
pub(crate) fn reaffirm_decision<P: LedgerProvider>(
    provider: &P,
    args: ReaffirmDecisionArgs,
) -> Result<ToolOutput, CoreError> {
    let handle = provider.ledger()?;
    let target = resolve_target(
        &handle,
        args.decision_id.as_deref(),
        args.description.as_deref(),
        args.topic.as_deref(),
        "decision_id",
    )?;
    let decision_id = match target {
        ResolvedTarget::Id(id) => id,
        ResolvedTarget::Ambiguous(output) => return Ok(output),
        ResolvedTarget::NotFound(output) => return Ok(output),
    };

    let graph = handle.graph().map_err(CoreError::from)?;
    let outcome = reaffirm(
        &handle.ledger,
        &handle.tenant_id,
        EventProvenance::agent(args.actor_id.clone()),
        &graph,
        match &args.statement {
            Statement::Own { reason } => ReaffirmCall {
                actor_id: &args.actor_id,
                decision_id: &decision_id,
                premise: args.premise_id.as_deref(),
                reason,
                relay: None,
            },
            Statement::Relayed(words) => ReaffirmCall {
                actor_id: &words.person,
                decision_id: &decision_id,
                premise: args.premise_id.as_deref(),
                reason: &words.quote,
                relay: Some(Relay::new(&args.actor_id, words.said_in.as_deref())),
            },
        },
    )
    .map_err(CoreError::from)?;
    match outcome {
        ReaffirmOutcome::Recorded(reply) => serde_json::to_value(reply)
            .map(ToolOutput)
            .map_err(|error| CoreError::Internal(error.to_string())),
        ReaffirmOutcome::Ambiguous(ambiguous) => Ok(ToolOutput(ambiguous.envelope())),
        ReaffirmOutcome::Refused(reason) => Err(CoreError::InvalidArgument(reason)),
    }
}

/// Parsed, validated arguments for the `get_reexamine_items` tool.
pub(crate) struct GetReexamineItemsArgs {
    pub(crate) decision_id: Option<String>,
    pub(crate) description: Option<String>,
    pub(crate) topic: Option<String>,
    pub(crate) premise_id: Option<String>,
    pub(crate) project: Option<String>,
    pub(crate) include_closed: bool,
    pub(crate) limit: usize,
    pub(crate) cursor: Option<String>,
}

impl GetReexamineItemsArgs {
    pub(crate) fn from_json(args: &Map<String, Value>) -> Result<Self, CoreError> {
        Ok(Self {
            decision_id: optional_string(args, "decision_id")?,
            description: optional_string(args, "description")?,
            topic: optional_string(args, "topic")?,
            premise_id: optional_string(args, "premise_id")?,
            project: optional_string(args, "project")?,
            include_closed: optional_bool_or(args, "include_closed", false)?,
            limit: optional_usize(args, "limit")?.unwrap_or(DEFAULT_REEXAMINE_PAGE_SIZE),
            cursor: optional_string(args, "cursor")?,
        })
    }
}

/// The core for the `get_reexamine_items` MCP tool (the CLI's `reexamine`): the decisions that
/// rest on something that changed and have not been looked at again, one page at a time.
/// `decision_id` or `description` narrows to one decision's items (resolved like every fluent
/// tool: an ambiguous or unmatched description is returned as-is); with neither, every decision
/// in the tenant (or the `project`'s scope) is covered. `data.truncated` / `data.next_cursor`
/// continue a long list; nothing is dropped silently.
pub(crate) fn get_reexamine_items<P: LedgerProvider>(
    provider: &P,
    graph: &impl GraphView,
    args: GetReexamineItemsArgs,
) -> Result<ToolOutput, CoreError> {
    let handle = provider.ledger()?;
    let dependent_id = if args.decision_id.is_some() || args.description.is_some() {
        let target = resolve_target(
            &handle,
            args.decision_id.as_deref(),
            args.description.as_deref(),
            args.topic.as_deref(),
            "decision_id",
        )?;
        match target {
            ResolvedTarget::Id(id) => Some(id),
            ResolvedTarget::Ambiguous(output) => return Ok(output),
            ResolvedTarget::NotFound(output) => return Ok(output),
        }
    } else {
        None
    };
    let request = ReexamineRequest {
        dependent_id,
        premise_id: args.premise_id,
        project: args.project,
        include_closed: args.include_closed,
        limit: args.limit,
        cursor: args.cursor,
        ..ReexamineRequest::default()
    };
    let context = QueryContext::new(handle.tenant_id.clone());
    let response = query_get_reexamine_items(&context, graph, &handle.ledger, &request)
        .map_err(CoreError::from)?;
    query_output(&response)
}

// ---------------------------------------------------------------------------
// score_decision / scan_decision_quality
// ---------------------------------------------------------------------------

/// The success payload of a read tool: the `QueryResponse` envelope every read tool returns.
/// For the quality tools the response is built by [`crate::quality_profile::report`], the core the
/// CLI shares, so the two transports and the CLI serialize the same value.
fn query_output<T: serde::Serialize>(
    response: &crate::queries::QueryResponse<T>,
) -> Result<ToolOutput, CoreError> {
    serde_json::to_value(response)
        .map(ToolOutput)
        .map_err(|error| CoreError::Internal(error.to_string()))
}

/// Parsed, validated arguments for the `score_decision` tool.
pub(crate) struct ScoreDecisionArgs {
    pub(crate) decision_id: String,
}

impl ScoreDecisionArgs {
    pub(crate) fn from_json(args: &Map<String, Value>) -> Result<Self, CoreError> {
        Ok(Self {
            decision_id: require_string(args, "decision_id")?,
        })
    }
}

/// The core for the `score_decision` MCP tool: the decision's quality profile (seven dimensions,
/// each with its level and reasons or why it was not assessed; attention lines; provenance), or
/// `data: null` when the decision does not exist. No number in it says how good the decision is.
pub(crate) fn score_decision(
    graph: &impl GraphView,
    args: ScoreDecisionArgs,
) -> Result<ToolOutput, CoreError> {
    let response =
        quality_profile::score_decision(graph, &args.decision_id).map_err(CoreError::from)?;
    query_output(&response)
}

/// Parsed, validated arguments for the `scan_decision_quality` tool.
pub(crate) struct ScanDecisionQualityArgs {
    pub(crate) request: ScanRequest,
}

impl ScanDecisionQualityArgs {
    pub(crate) fn from_json(args: &Map<String, Value>) -> Result<Self, CoreError> {
        let kinds = parse_kinds(&optional_string_array(args, "kinds")?)
            .map_err(CoreError::InvalidArgument)?;
        let evidence_window_days = optional_usize(args, "evidence_window_days")?
            .map(|days| {
                u32::try_from(days).map_err(|_| {
                    CoreError::InvalidArgument(
                        "`evidence_window_days` is too large: at most 4294967295".to_owned(),
                    )
                })
            })
            .transpose()?;
        Ok(Self {
            request: ScanRequest {
                kinds,
                limit: optional_usize(args, "limit")?.unwrap_or(SCAN_DEFAULT_LIMIT),
                cursor: optional_string(args, "cursor")?,
                evidence_window_days,
            },
        })
    }
}

/// The core for the `scan_decision_quality` MCP tool: one page of attention findings, each with
/// the dimensions it bears on. `truncated` says whether more follow and `data.next_cursor`
/// resumes; nothing is dropped silently and no request returns the whole graph.
pub(crate) fn scan_decision_quality(
    graph: &impl GraphView,
    args: ScanDecisionQualityArgs,
) -> Result<ToolOutput, CoreError> {
    let response =
        quality_profile::scan_decision_quality(graph, &args.request).map_err(CoreError::from)?;
    query_output(&response)
}

/// Parsed, validated arguments for the `get_suggestions` tool: the arguments of
/// `scan_decision_quality`, parsed and refused the same way, and `exclude_acknowledged`.
pub(crate) struct GetSuggestionsArgs {
    pub(crate) request: SuggestionsRequest,
}

impl GetSuggestionsArgs {
    pub(crate) fn from_json(args: &Map<String, Value>) -> Result<Self, CoreError> {
        Ok(Self {
            request: SuggestionsRequest {
                scan: ScanDecisionQualityArgs::from_json(args)?.request,
                exclude_acknowledged: optional_bool_or(args, "exclude_acknowledged", true)?,
            },
        })
    }
}

/// The core for the `get_suggestions` MCP tool: one page of attention findings, without the ones
/// someone has acknowledged unless `exclude_acknowledged` is false. The page is filled from what
/// remains, `truncated` says whether more follow and `data.next_cursor` resumes.
pub(crate) fn get_suggestions(
    graph: &impl GraphView,
    args: GetSuggestionsArgs,
) -> Result<ToolOutput, CoreError> {
    let response =
        quality_profile::get_suggestions(graph, &args.request).map_err(CoreError::from)?;
    query_output(&response)
}

/// Parsed, validated arguments for the `rank_decisions_by_importance` tool.
pub(crate) struct RankDecisionsByImportanceArgs {
    pub(crate) request: ImportanceRequest,
}

impl RankDecisionsByImportanceArgs {
    pub(crate) fn from_json(args: &Map<String, Value>) -> Result<Self, CoreError> {
        Ok(Self {
            request: ImportanceRequest {
                include_not_in_force: optional_bool_or(args, "include_not_in_force", false)?,
                limit: optional_usize(args, "limit")?.unwrap_or(DEFAULT_PAGE_SIZE),
                cursor: optional_string(args, "cursor")?,
            },
        })
    }
}

/// The core for the `rank_decisions_by_importance` MCP tool: one page of the decisions ordered by
/// what rests on them, each with who decided it. `truncated` says whether more follow and
/// `data.next_cursor` resumes; a decision the record gives no reason to rank reads
/// `not_assessed`, never a number. No model is called.
pub(crate) fn rank_decisions_by_importance(
    graph: &impl GraphView,
    args: RankDecisionsByImportanceArgs,
) -> Result<ToolOutput, CoreError> {
    let response =
        importance::rank_decisions_by_importance(graph, &args.request).map_err(CoreError::from)?;
    query_output(&response)
}

// ---------------------------------------------------------------------------
// recent_decisions / decision_quality_candidates / get_decision_context /
// decision_context_candidates / scan_misfiled_decisions / analyze_failure_modes
// ---------------------------------------------------------------------------

/// A page of a cursor-paged read tool: the [`query_output`] envelope plus `next_cursor`, which
/// resumes the walk when the page was `truncated` and is `null` otherwise. `scanned` is how many
/// rows the page looked at, which is `result_count` unless the query drops rows after reading them.
/// `next_cursor_of` is the cursor helper of the tool's own query module, so a tool keeps the cursor
/// its query defines.
fn paged_output<T: serde::Serialize>(
    response: &QueryResponse<T>,
    cursor: Option<&str>,
    scanned: usize,
    next_cursor_of: fn(usize, usize) -> Option<String>,
) -> Result<ToolOutput, CoreError> {
    let skip: usize = cursor.and_then(|c| c.parse().ok()).unwrap_or(0);
    let next_cursor = if response.truncated {
        next_cursor_of(skip, scanned)
    } else {
        None
    };
    Ok(ToolOutput(json!({
        "result_count": response.result_count,
        "truncated": response.truncated,
        "latency_ms": response.latency_ms,
        "next_cursor": next_cursor,
        "data": response.data,
    })))
}

/// The `since_event_origin` filter the candidate and attribution tools share. A value that is not
/// an integer is left out, not refused: what these tools have always done on stdio.
fn since_event_origin(args: &Map<String, Value>) -> Option<i64> {
    args.get("since_event_origin").and_then(Value::as_i64)
}

/// Parsed, validated arguments for the `recent_decisions` tool.
pub(crate) struct RecentDecisionsArgs {
    pub(crate) request: RecentDecisionsRequest,
}

impl RecentDecisionsArgs {
    pub(crate) fn from_json(args: &Map<String, Value>) -> Result<Self, CoreError> {
        let since_timestamp = optional_datetime(args, "since")?
            .ok_or_else(|| CoreError::InvalidArgument("missing `since`".to_owned()))?;
        let statuses = optional_string_array(args, "status")?
            .iter()
            .map(|status| parse_decision_status(status))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            request: RecentDecisionsRequest {
                since_timestamp,
                until_timestamp: optional_datetime(args, "until")?,
                filters: RecentDecisionFilterRequest {
                    actor_patterns: optional_string_array(args, "actor")?,
                    sources: optional_string_array(args, "source")?,
                    topic_keys: optional_string_array(args, "topic")?,
                    statuses,
                },
                limit: optional_usize(args, "limit")?.unwrap_or(25),
                cursor: optional_string(args, "cursor")?,
            },
        })
    }
}

/// The core for the `recent_decisions` MCP tool: one bounded page of the decisions decided in a
/// window. The ledger's proposals are read as events; the decisions a classifier drafted from a
/// transcript are known to the graph alone, so it takes both.
pub(crate) fn recent_decisions<P: LedgerProvider>(
    provider: &P,
    graph: &impl GraphView,
    args: RecentDecisionsArgs,
) -> Result<ToolOutput, CoreError> {
    let handle = provider.ledger()?;
    let response =
        get_recent_decisions(&handle.ledger, graph, &args.request).map_err(CoreError::from)?;
    query_output(&response)
}

/// Parsed, validated arguments for the `decision_quality_candidates` tool.
pub(crate) struct DecisionQualityCandidatesArgs {
    pub(crate) request: DecisionQualityCandidatesRequest,
}

impl DecisionQualityCandidatesArgs {
    pub(crate) fn from_json(args: &Map<String, Value>) -> Result<Self, CoreError> {
        Ok(Self {
            request: DecisionQualityCandidatesRequest {
                since_event_origin: since_event_origin(args),
                limit: optional_usize(args, "limit")?.unwrap_or(25),
                cursor: optional_string(args, "cursor")?,
                only_with_signals: args
                    .get("only_with_signals")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            },
        })
    }
}

/// The core for the `decision_quality_candidates` MCP tool: one page of decisions with their
/// outcome signals, `next_cursor` set when more follow. A truncated page examined a full window of
/// decisions, so the cursor moves past that window, however few of its decisions the page kept.
pub(crate) fn decision_quality_candidates(
    graph: &impl GraphView,
    args: DecisionQualityCandidatesArgs,
) -> Result<ToolOutput, CoreError> {
    let response =
        get_decision_quality_candidates(graph, &args.request).map_err(CoreError::from)?;
    paged_output(
        &response,
        args.request.cursor.as_deref(),
        outcome_page_len(&args.request),
        outcome_next_cursor,
    )
}

/// Parsed, validated arguments for the `get_decision_context` tool.
pub(crate) struct GetDecisionContextArgs {
    pub(crate) decision_id: String,
}

impl GetDecisionContextArgs {
    pub(crate) fn from_json(args: &Map<String, Value>) -> Result<Self, CoreError> {
        Ok(Self {
            decision_id: require_string(args, "decision_id")?,
        })
    }
}

/// The core for the `get_decision_context` MCP tool: the authorship and review shape of one
/// decision, or `data: null` when the decision does not exist.
pub(crate) fn get_decision_context(
    graph: &impl GraphView,
    args: GetDecisionContextArgs,
) -> Result<ToolOutput, CoreError> {
    let response = query_get_decision_context(graph, &args.decision_id).map_err(CoreError::from)?;
    query_output(&response)
}

/// Parsed, validated arguments for the `decision_context_candidates` tool.
pub(crate) struct DecisionContextCandidatesArgs {
    pub(crate) request: DecisionContextRequest,
}

impl DecisionContextCandidatesArgs {
    pub(crate) fn from_json(args: &Map<String, Value>) -> Result<Self, CoreError> {
        Ok(Self {
            request: DecisionContextRequest {
                since_event_origin: since_event_origin(args),
                limit: optional_usize(args, "limit")?.unwrap_or(25),
                cursor: optional_string(args, "cursor")?,
            },
        })
    }
}

/// The core for the `decision_context_candidates` MCP tool: one page of decision contexts,
/// `next_cursor` set when more follow.
pub(crate) fn decision_context_candidates(
    graph: &impl GraphView,
    args: DecisionContextCandidatesArgs,
) -> Result<ToolOutput, CoreError> {
    let response =
        get_decision_context_candidates(graph, &args.request).map_err(CoreError::from)?;
    paged_output(
        &response,
        args.request.cursor.as_deref(),
        response.result_count,
        context_next_cursor,
    )
}

/// Parsed, validated arguments for the `scan_misfiled_decisions` tool.
pub(crate) struct ScanMisfiledDecisionsArgs {
    pub(crate) request: MisfiledScanRequest,
}

impl ScanMisfiledDecisionsArgs {
    pub(crate) fn from_json(args: &Map<String, Value>) -> Result<Self, CoreError> {
        Ok(Self {
            request: MisfiledScanRequest {
                foreign_topic_keys: require_string_array(args, "foreign_topic_keys")?,
                project: optional_string(args, "project")?,
                move_to: optional_string(args, "move_to")?,
                limit: optional_usize(args, "limit")?.unwrap_or(25),
                cursor: optional_string(args, "cursor")?,
            },
        })
    }
}

/// The core for the `scan_misfiled_decisions` MCP tool: one page of decisions carrying a topic key
/// the caller named as foreign to this ledger, `next_cursor` set when more follow. A report only:
/// nothing is moved.
///
/// The scan reads the graph, which has no project registry, so a `project` or `move_to` handle is
/// checked against the ledger first: a typo is a refusal, never "nothing misfiled".
pub(crate) fn scan_misfiled_decisions<P: LedgerProvider>(
    provider: &P,
    graph: &impl GraphView,
    args: ScanMisfiledDecisionsArgs,
) -> Result<ToolOutput, CoreError> {
    if args.request.project.is_some() || args.request.move_to.is_some() {
        let handle = provider.ledger()?;
        let scoped = TenantScopedLedger::new(&handle.ledger, handle.tenant_id.clone());
        for project in args
            .request
            .project
            .iter()
            .chain(args.request.move_to.iter())
        {
            require_registered_project(&scoped, project).map_err(CoreError::from)?;
        }
    }
    let response = query_scan_misfiled_decisions(graph, &args.request).map_err(CoreError::from)?;
    paged_output(
        &response,
        args.request.cursor.as_deref(),
        response.result_count,
        misfiled_next_cursor,
    )
}

/// Parsed, validated arguments for the `analyze_failure_modes` tool.
pub(crate) struct AnalyzeFailureModesArgs {
    pub(crate) request: FailureAttributionRequest,
}

impl AnalyzeFailureModesArgs {
    pub(crate) fn from_json(args: &Map<String, Value>) -> Result<Self, CoreError> {
        Ok(Self {
            request: FailureAttributionRequest {
                since_event_origin: since_event_origin(args),
                min_sample_size: optional_usize(args, "min_sample_size")?.unwrap_or(3),
            },
        })
    }
}

/// The core for the `analyze_failure_modes` MCP tool: aggregate failure-rate patterns across
/// authorship, review, source, context richness and the seven quality dimensions; only groups of
/// at least `min_sample_size` decisions are listed as findings.
pub(crate) fn analyze_failure_modes(
    graph: &impl GraphView,
    args: AnalyzeFailureModesArgs,
) -> Result<ToolOutput, CoreError> {
    let response =
        quality_profile::analyze_failure_modes(graph, &args.request).map_err(CoreError::from)?;
    query_output(&response)
}

// ---------------------------------------------------------------------------
// capture_evidence / capture_hypothesis
// ---------------------------------------------------------------------------

/// Parsed, validated arguments for the `capture_evidence` tool.
pub(crate) struct CaptureEvidenceArgs {
    pub(crate) actor_id: String,
    pub(crate) content: String,
}

impl CaptureEvidenceArgs {
    pub(crate) fn from_json(
        args: &Map<String, Value>,
        actor_id: String,
    ) -> Result<Self, CoreError> {
        Ok(Self {
            actor_id,
            content: require_string(args, "content")?,
        })
    }
}

/// The core for the `capture_evidence` MCP tool: records one piece of evidence on its own, with
/// no decision resting on it yet.
pub(crate) fn capture_evidence<P: LedgerProvider>(
    provider: &P,
    args: CaptureEvidenceArgs,
) -> Result<ToolOutput, CoreError> {
    let handle = provider.ledger()?;
    let commands = Commands::new_with_context(
        &handle.ledger,
        CommandContext::new(
            handle.tenant_id,
            EventProvenance::agent(args.actor_id.clone()),
        ),
    );
    let evidence_id = commands
        .record_evidence(&args.actor_id, &args.content)
        .map_err(CoreError::from)?;
    Ok(ToolOutput(json!({ "evidence_id": evidence_id })))
}

/// Parsed, validated arguments for the `capture_hypothesis` tool.
pub(crate) struct CaptureHypothesisArgs {
    pub(crate) actor_id: String,
    pub(crate) statement: String,
}

impl CaptureHypothesisArgs {
    pub(crate) fn from_json(
        args: &Map<String, Value>,
        actor_id: String,
    ) -> Result<Self, CoreError> {
        Ok(Self {
            actor_id,
            statement: require_string(args, "statement")?,
        })
    }
}

/// The core for the `capture_hypothesis` MCP tool: records one assumption on its own, with no
/// decision resting on it yet.
pub(crate) fn capture_hypothesis<P: LedgerProvider>(
    provider: &P,
    args: CaptureHypothesisArgs,
) -> Result<ToolOutput, CoreError> {
    let handle = provider.ledger()?;
    let commands = Commands::new_with_context(
        &handle.ledger,
        CommandContext::new(
            handle.tenant_id,
            EventProvenance::agent(args.actor_id.clone()),
        ),
    );
    let hypothesis_id = commands
        .record_hypothesis(&args.actor_id, &args.statement)
        .map_err(CoreError::from)?;
    Ok(ToolOutput(json!({ "hypothesis_id": hypothesis_id })))
}

// ---------------------------------------------------------------------------
// get_decision / get_relevant_decisions / search_decisions / dump_graph /
// summarize_decisions
// ---------------------------------------------------------------------------

/// Parsed, validated arguments for the `get_decision` tool.
pub(crate) struct GetDecisionArgs {
    pub(crate) decision_id: String,
}

impl GetDecisionArgs {
    pub(crate) fn from_json(args: &Map<String, Value>) -> Result<Self, CoreError> {
        Ok(Self {
            decision_id: require_string(args, "decision_id")?,
        })
    }
}

/// The core for the `get_decision` MCP tool: one decision as the graph holds it, or `data: null`
/// when no decision has the id.
pub(crate) fn get_decision(
    graph: &impl GraphView,
    args: GetDecisionArgs,
) -> Result<ToolOutput, CoreError> {
    let response = query_get_decision(graph, &args.decision_id).map_err(CoreError::from)?;
    query_output(&response)
}

/// Parsed, validated arguments for the `get_relevant_decisions` tool.
pub(crate) struct GetRelevantDecisionsArgs {
    pub(crate) topic: String,
    pub(crate) status: Option<DecisionStatus>,
}

impl GetRelevantDecisionsArgs {
    pub(crate) fn from_json(args: &Map<String, Value>) -> Result<Self, CoreError> {
        Ok(Self {
            topic: require_string(args, "topic")?,
            status: optional_string(args, "status")?
                .map(|status| parse_decision_status(&status))
                .transpose()?,
        })
    }
}

/// The core for the `get_relevant_decisions` MCP tool: the decisions filed under a topic key,
/// optionally only those in one status. A `status` that is not a status name is refused, never
/// read as "any".
pub(crate) fn get_relevant_decisions(
    graph: &impl GraphView,
    args: GetRelevantDecisionsArgs,
) -> Result<ToolOutput, CoreError> {
    let response =
        query_get_relevant_decisions(graph, &args.topic, args.status).map_err(CoreError::from)?;
    query_output(&response)
}

/// Default page size of `search_decisions`.
const SEARCH_DEFAULT_LIMIT: usize = 25;

/// Parsed, validated arguments for the `search_decisions` tool.
pub(crate) struct SearchDecisionsArgs {
    pub(crate) request: SearchDecisionRequest,
}

impl SearchDecisionsArgs {
    pub(crate) fn from_json(args: &Map<String, Value>) -> Result<Self, CoreError> {
        let query = optional_string(args, "q")?;
        let statuses = optional_string_array(args, "status")?
            .iter()
            .map(|status| parse_decision_status(status))
            .collect::<Result<Vec<_>, _>>()?;
        let limit = optional_usize(args, "limit")?.unwrap_or(SEARCH_DEFAULT_LIMIT);
        Ok(Self {
            request: SearchDecisionRequest {
                query,
                topic_keys: optional_string_array(args, "topic")?,
                statuses,
                actor_ids: optional_string_array(args, "actor_id")?,
                sources: optional_string_array(args, "source")?,
                since: optional_datetime(args, "since")?,
                until: optional_datetime(args, "until")?,
                limit,
                cursor: optional_string(args, "cursor")?,
                project: None,
            },
        })
    }
}

/// The core for the `search_decisions` MCP tool: one page of the decisions matching a full-text
/// query and filters. The ledger must be [`AnyLedger`] specifically: the full-text search picks
/// its index by backend.
pub(crate) fn search_decisions<P>(
    provider: &P,
    graph: &impl GraphView,
    args: SearchDecisionsArgs,
) -> Result<ToolOutput, CoreError>
where
    P: LedgerProvider<Ledger = AnyLedger>,
{
    let handle = provider.ledger()?;
    let context = QueryContext::new(handle.tenant_id);
    let response = search_decisions_any(&context, &handle.ledger, graph, &args.request)
        .map_err(CoreError::from)?;
    query_output(&response)
}

/// The core for the `dump_graph` MCP tool: the decision graph as Graphviz DOT. Takes no
/// arguments.
pub(crate) fn dump_graph(graph: &impl GraphView) -> Result<ToolOutput, CoreError> {
    let dot = crate::cli::render_decision_dot(graph).map_err(CoreError::from)?;
    Ok(ToolOutput(json!({ "format": "dot", "content": dot })))
}

/// Parsed, validated arguments for the `summarize_decisions` tool.
pub(crate) struct SummarizeDecisionsArgs {
    pub(crate) request: SummarizeRequest,
}

impl SummarizeDecisionsArgs {
    pub(crate) fn from_json(args: &Map<String, Value>) -> Result<Self, CoreError> {
        let decision_ids = require_string_array(args, "decision_ids")?;
        if decision_ids.is_empty() {
            return Err(CoreError::InvalidArgument(
                "decision_ids must not be empty".to_owned(),
            ));
        }
        let refuse_unless_one = |mode: &str| {
            if decision_ids.len() == 1 {
                Ok(())
            } else {
                Err(CoreError::InvalidArgument(format!(
                    "mode={mode} requires exactly one decision_id"
                )))
            }
        };
        let mode = match optional_string(args, "mode")?.as_deref() {
            None if decision_ids.len() == 1 => SummarizeMode::Single,
            None => SummarizeMode::Cluster,
            Some("single") => {
                refuse_unless_one("single")?;
                SummarizeMode::Single
            }
            Some("cluster") => SummarizeMode::Cluster,
            Some("chain") => {
                refuse_unless_one("chain")?;
                SummarizeMode::Chain
            }
            Some(other) => {
                return Err(CoreError::InvalidArgument(format!(
                    "unknown mode `{other}`; must be single, cluster, or chain"
                )))
            }
        };
        Ok(Self {
            request: SummarizeRequest { decision_ids, mode },
        })
    }
}

/// The core for the `summarize_decisions` MCP tool: a deterministic digest of one decision, a
/// cluster of them, or one decision's supersession chain. No model is called.
pub(crate) fn summarize_decisions(
    graph: &impl GraphView,
    args: SummarizeDecisionsArgs,
) -> Result<ToolOutput, CoreError> {
    let response =
        crate::summarize::summarize_decisions(graph, &args.request).map_err(CoreError::from)?;
    query_output(&response)
}

// ---------------------------------------------------------------------------
// classify_queue_list / classify_queue_submit (HTTP only, see HTTP_ONLY_TOOLS)
// ---------------------------------------------------------------------------

/// Default page size of `classify_queue_list`.
const CLASSIFY_QUEUE_DEFAULT_LIMIT: usize = 20;

/// The classifier recorded on a submission that names none: the subscription-seat worker.
const CLASSIFY_QUEUE_DEFAULT_MODEL: &str = "agent:worker-a";

/// Parsed, validated arguments for the `classify_queue_list` tool.
pub(crate) struct ClassifyQueueListArgs {
    pub(crate) session_id: Option<String>,
    pub(crate) limit: usize,
}

impl ClassifyQueueListArgs {
    pub(crate) fn from_json(args: &Map<String, Value>) -> Result<Self, CoreError> {
        Ok(Self {
            session_id: optional_string(args, "session_id")?,
            limit: optional_usize(args, "limit")?.unwrap_or(CLASSIFY_QUEUE_DEFAULT_LIMIT),
        })
    }
}

/// The core for the `classify_queue_list` MCP tool: the oldest pending (unclassified) ingest
/// batches with rendered turn text, plus today's classification budget; `pending_total` and
/// `truncated` say how much more is waiting. The MCP mirror of `GET /v1/classify-queue`
/// (hivemind-zdsh.18).
pub(crate) fn classify_queue_list<P: LedgerProvider>(
    provider: &P,
    args: ClassifyQueueListArgs,
) -> Result<ToolOutput, CoreError> {
    let handle = provider.ledger()?;
    let queue = crate::classifier::pending_queue_for_ledger(
        &handle.ledger,
        &handle.tenant_id,
        args.session_id.as_deref(),
        args.limit,
    )
    .map_err(CoreError::from)?;
    Ok(ToolOutput(queue.to_json()))
}

/// Parsed, validated arguments for the `classify_queue_submit` tool.
pub(crate) struct ClassifyQueueSubmitArgs {
    pub(crate) actor_id: String,
    pub(crate) batch_ids: Vec<String>,
    pub(crate) captures: Vec<CaptureItem>,
    pub(crate) model: String,
}

impl ClassifyQueueSubmitArgs {
    pub(crate) fn from_json(
        args: &Map<String, Value>,
        actor_id: String,
    ) -> Result<Self, CoreError> {
        let batch_ids = require_string_array(args, "batch_ids")?;
        if batch_ids.is_empty() {
            return Err(CoreError::InvalidArgument(
                "batch_ids must not be empty".to_owned(),
            ));
        }
        let captures = args
            .get("captures")
            .cloned()
            .ok_or_else(|| CoreError::InvalidArgument("missing `captures`".to_owned()))?;
        let captures = serde_json::from_value(captures).map_err(|error| {
            CoreError::InvalidArgument(format!("`captures` is invalid: {error}"))
        })?;
        Ok(Self {
            actor_id,
            batch_ids,
            captures,
            model: optional_string(args, "model")?
                .unwrap_or_else(|| CLASSIFY_QUEUE_DEFAULT_MODEL.to_owned()),
        })
    }
}

/// The core for the `classify_queue_submit` MCP tool: records the captures for one or more
/// pending batches from the same session as a single classification event, the session-grouped
/// cadence (hivemind-zdsh.18): one model call per session end, not one per batch. A batch the
/// tenant never received is refused by name with nothing written, and once the daily cap is hit
/// the submission is refused, not dropped: the batches stay pending for the next day. The MCP
/// mirror of `POST /v1/classify-queue/submit`.
pub(crate) fn classify_queue_submit<P: LedgerProvider>(
    provider: &P,
    args: ClassifyQueueSubmitArgs,
) -> Result<ToolOutput, CoreError> {
    let handle = provider.ledger()?;
    let commands = Commands::new_with_context(
        &handle.ledger,
        CommandContext::new(
            handle.tenant_id.clone(),
            EventProvenance::agent(args.actor_id.clone()),
        ),
    );

    // A batch this tenant never received is a bad argument whatever the cap says.
    commands
        .require_received_batches(&args.batch_ids)
        .map_err(CoreError::from)?;

    let cap_status = crate::classifier::daily_cap_status(&handle.ledger, &handle.tenant_id)
        .map_err(CoreError::from)?;
    if cap_status.remaining == 0 {
        return Err(CoreError::InvalidArgument(format!(
            "daily classification cap ({}) reached; batches remain pending until tomorrow (UTC)",
            cap_status.cap
        )));
    }

    let recorded = commands
        .record_ingest_batch_classified(
            &args.actor_id,
            &args.batch_ids,
            &args.model,
            crate::classifier::SCHEMA_VERSION,
            args.captures,
            None,
        )
        .map_err(CoreError::from)?;

    let mut reply = json!({
        "batch_ids": args.batch_ids,
        "capture_count": recorded.recorded_count,
        "event_id": recorded.event_id,
    });
    recorded.annotate_reply(&mut reply);
    Ok(ToolOutput(reply))
}
