//! `why` and `verify` give the same answer through every surface (hivemind-ctok).
//!
//! One resolver reads the description for the CLI, the HTTP API and the MCP server, and the
//! plugin's `why.sh` / `verify.sh` call the CLI. Each of them still calls the resolver itself and
//! builds its own reply, and nothing but this test stops one from answering with another decision,
//! listing where the others answer, or leaving off the `close_match` line that says what the
//! decision lacks. The questions are the ones the after-deploy check asked when the resolver
//! answered an unrelated decision in one call (hivemind-tfde) or gave a different answer to the
//! same words as `recall` (hivemind-3lko), put to a ledger shaped like the one they were asked of:
//! decisions whose long rationales hold the question's words by chance, next to the decision whose
//! title says them.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use clap::Parser;
use http_body_util::BodyExt as _;
use serde_json::{json, Value};
use tower::ServiceExt as _;

use loomtracer::cli::{run, Cli};

#[allow(dead_code)]
#[path = "support/seed_data.rs"]
mod seed_data;

use seed_data::{unique_temp_dir, TestResult};

/// (label, title, topic keys, rationale). The labels are what the checks below say; the decision
/// ids are made when the ledger is seeded.
const DECISIONS: &[(&str, &str, &str, &str)] = &[
    (
        "hosted",
        "The paid shape is picked after the comparison and the testers' feedback; the waitlist price question replaces interviews",
        "pricing,packaging,hosted,waitlist",
        "The paid shape is chosen after the comparison run and the first testers' feedback, not after interviews. The waitlist price question replaces the interviews, because asking a price is cheaper than scheduling a call. Until then the hosted cell stays on one node and nobody is charged. The comparison covers a hosted plan with self-hosting free against one Pro plan valid hosted or self-hosted.",
    ),
    (
        "paywall",
        "Nothing is paywalled until after the own-use trial; self-hosting stays free, a paid offer is hosted service and support",
        "pricing,packaging,licensing,self-hosting,hosted",
        "Read as: nothing is paywalled until after the own-use trial, and the self-hosted version stays free, so any future paid offer is the hosted service and support, not a self-host licence. A hosted plan with self-hosting free is one candidate shape; one Pro plan valid hosted or self-hosted is the other. The trial decides which plan to charge for, and the hosted cell is not billed during it. The maintainer sets the price after the trial.",
    ),
    (
        "gate",
        "Triage gate built against the System One shape, local model default, opt-in for the hosted classifier",
        "classifier,triage-gate,privacy,self-hosted",
        "The triage gate sits in front of the classifier that turns transcripts into decision records. It answers yes or no questions: is there a decision here at all, does it bind something or cost to undo, which existing question does this answer, which project. Today a model call per excerpt is spent to learn that most excerpts hold nothing, capped at 150 calls a day. A local model is the default; the service is still called by its working name in the gate prompts, and nothing is sent to the hosted classifier without opt-in.",
    ),
    (
        "name",
        "The service's new name is Atlas, home domain atlas.example, replacing the working name (not Beacon, not Compass)",
        "service-name,naming,rename,domain",
        "A, Atlas. Home domain atlas.example; the domain is bought by the maintainer; nothing is registered or reserved; the rename itself is not planned or started. Why the working name goes: it is overloaded and taken, on the main registries and with dozens of repositories of that exact name. Options: A = Atlas, a decision is mapped or it is not; B = Beacon; C = Compass. The site and the docs move to the new domain at the rename, one word at a time.",
    ),
    (
        "slugs-browser",
        "Decision links use title slugs worked out the same in every browser, not slugs remembered per browser",
        "ui,links,routing",
        "The server keeps no slug, and titles can change. Showing a slug remembered by one browser would let two browsers give one link to different decisions, so a shared link could open the wrong decision. Deterministic slugs open the same decision everywhere. Old links survive a retitle through the id suffix.",
    ),
    (
        "slugs-server",
        "UI decision links use the server's stable slug; title-derived slugs only when the server sends none",
        "ui,links,slugs",
        "The server now assigns each decision a slug once at proposal and keeps it through a retitle, and serves it on the graph route. Deriving links from titles in the browser moved a link on every retitle and needed a per-browser memory to keep old links working. The UI takes the server's slug first and works a slug out from the title only for decisions the server sent none for, such as the public demo snapshot.",
    ),
    (
        "flow-titles",
        "Flow node titles are laid out in screen space so none overprints: nudge, then leader line, else hide",
        "ui,flow,labels,layout",
        "Titles are overlays anchored above each node; on a phone fourteen titles share a narrow band. A greedy placer keeps each title on its spot if free, else the nearest free nudge with a faint leader line back to its node, else hides it; key titles first, then birth order.",
    ),
    (
        "flow-card",
        "On a phone, keep the Flow info card's room only while a card is pinned",
        "ui,flow,labels,phone",
        "The fixed room for the info card keeps titles from printing over any card, and costs nothing on a desktop, but on a small phone canvas it is a quarter of the canvas and every title that loses its spot is hidden.",
    ),
    (
        "demo-copy",
        "Public demo data: commit a byte-exact copy of the generator export, checked by recorded SHA-256",
        "ui,public-demo,example-data",
        "The demo build must work without a loomtracer checkout (CI, the site rebuild), so the generator's graph and briefs files are committed rather than read from loomtracer at build time. A sync script copies them byte for byte and records the source commit and each file's hash; a test fails if a copy no longer matches.",
    ),
    (
        "demo-runtime",
        "The public demo reads its example snapshot at runtime as static files in the server's own shapes",
        "ui,public-demo,snapshot",
        "The read-only public demo build fetches the graph and briefs files from a snapshot folder at runtime, mapped by the same code that reads a live server. The generator exports exactly what the server returns, so nothing is derived in the browser.",
    ),
    (
        "site-label",
        "Site graphs label the supersedes edge 'replaces' and draw coming-next parts solid with a COMING NEXT mark",
        "website,graphs",
        "The bead lists replaces among the product's graph words and the page says replace in plain words everywhere, while the app's graph reads supersedes; the site uses replaces and the claims file records the difference. Coming-next parts are merged and built, so they are drawn solid with a COMING NEXT mark; only planned parts are dashed and marked PLANNED.",
    ),
    (
        "site-chain",
        "Site graph 'one assumption falls' shows refutation marking only direct dependents; the chain goes stale on replacement",
        "website,graphs,honesty",
        "The product marks a decision as not holding when an assumption it rests on directly is refuted, one hop. A decision that follows from such a decision is not marked until the decision in between is replaced, and then it reads stale. Drawing the whole chain lighting up on the refutation alone would show something neither built nor planned.",
    ),
    (
        "slack-author",
        "Slack reaction and shortcut captures: the message's author decides, whoever triggered it records",
        "slack,attribution,capture",
        "When someone reacts to or runs the shortcut on another person's Slack message, the words and the choice are the author's, so the author is recorded as the decider and the person who triggered the capture as the recorder. Recording the reactor as decider would claim a person accepted a decision someone else worded.",
    ),
    (
        "framing",
        "Interview and search-term framing leads with who decided what, why, and whether it held up",
        "positioning,framing",
        "The lead line for the interviews and search terms is who decided what, why, and whether it held up. A person who saves a message, adds a mark or counts votes is not the one who decided; the framing says the person, not the tool.",
    ),
    (
        "agent-name",
        "The UI names every agent 'an agent', never by the tool or role it ran as",
        "ui,naming",
        "An agent's actor id names the tool that ran it and its role in that tool: internal names a reader cannot make sense of. What a reader needs is whether a person or an agent decided, so every agent reads alike.",
    ),
    (
        "option-letters",
        "UI letters a shortened option from its recorded label, else its id, only when the options run A, B, C",
        "ui,decision-page,options",
        "The preview API shortens option labels and sends the recorded label only where the option was captured with a label of its own. The id is text the capturing agent wrote, not a guess, so reading it keeps the rule that no letter is invented.",
    ),
    (
        "status-history",
        "Decision page status history: one row per logged status change; the demo snapshot keeps its undated rows",
        "ui,status,decision-page",
        "The server's status-events read dates every proposal, acceptance, rejection and supersession and names who made it and the status it led to, so each history row is one logged change. Nothing is derived in the browser. The public demo's snapshot carries only the graph and the briefs, so it keeps the rows it showed before.",
    ),
    (
        "verdict-note",
        "A superseded decision that was contested says so on a second line of its one verdict note, naming who rejected it",
        "decision-page,verdict",
        "The decision page showed one verdict note and chose superseded before contested, so a contested first vote that was later superseded hid its disagreement. Keeping one note with a second line adds the contest without a new layout element and keeps the supersession first.",
    ),
    (
        "deck",
        "Pitch deck: the value and who it is for instead of install steps, graphs over the held-up shot, still eight slides",
        "pitch-deck,positioning",
        "The install commands go; the deck says the value and who it is for, described by values. Each of the eight slides is made fuller instead of adding new ones.",
    ),
    (
        "listing",
        "Listing order after proof: Claude Code plugin first, then MCP Registry, then community lists",
        "channels,distribution",
        "All three listings follow the own-use trial, in the order the plugin, the registry, the community lists. The registry is the next step right after having shown that it actually works.",
    ),
    (
        "related-layer",
        "UI shows possibly-related decisions as an opt-in Graph layer: dashed, no arrowhead, first five per decision",
        "ui,graph",
        "The layer is off by default, drawn dashed, muted and without arrowheads, joining the selected decision to the first page of five suggestions, so none reads as a recorded relation.",
    ),
    (
        "no-people",
        "The graph draws no people; who decided shows under the selected decision's title",
        "ui,graph,actors",
        "Showing relation words at rest would put the server's passive 'proposed by' and 'accepted by' on every actor edge, and there is no active verb that reads decision to actor along an arrow that runs newer to older. Who decided shows on the decision itself.",
    ),
];

/// What a question must come to, on every surface.
#[derive(Debug)]
enum Expect {
    /// Answered in one call with this decision.
    Answers(&'static str),
    /// Not answered: a list of candidates, the first of them this decision (the one asked about).
    ListsFirst(&'static str),
    /// Not answered: a list, whichever decision it starts with (no decision is about the question).
    Lists,
    /// The decision asked about comes first, whether the reply answers with it or lists it first.
    Leads(&'static str),
    /// No decision matches.
    NoMatch,
}

/// (verb, question, expected). The first six are the questions hivemind-tfde was filed on, the
/// next three the ones hivemind-3lko was, and the last two ask about something never decided: a
/// miss is an answer on every surface (hivemind-996m).
const CASES: &[(&str, &str, Expect)] = &[
    // hivemind-tfde: no decision is about Postgres or a per-seat charge, and the ones that share
    // some of the words must not be named as the answer.
    (
        "why",
        "why did we choose Postgres for the hosted cell?",
        Expect::Lists,
    ),
    (
        "why",
        "why do we charge per seat for the hosted plan?",
        Expect::Lists,
    ),
    (
        "why",
        "where do decision page addresses come from now, the browser or the server?",
        Expect::ListsFirst("slugs-browser"),
    ),
    (
        "why",
        "how are decision URLs built so every browser gets the same one?",
        Expect::Answers("slugs-browser"),
    ),
    (
        "why",
        "what word do the site's diagrams put on the arrow from a new decision to the one it replaced?",
        Expect::Leads("site-label"),
    ),
    (
        "why",
        "why is the demo's data checked into the UI repo instead of read from loomtracer when it builds?",
        Expect::Answers("demo-copy"),
    ),
    // hivemind-3lko: what recall names, why and verify name.
    (
        "why",
        "how are Flow node titles kept from overprinting?",
        Expect::Leads("flow-titles"),
    ),
    (
        "why",
        "how do we keep links to a decision page stable across browsers?",
        Expect::Leads("slugs-server"),
    ),
    (
        "verify",
        "is the service still called Atlas",
        Expect::Answers("name"),
    ),
    ("why", "what is the weather on mars", Expect::NoMatch),
    ("verify", "what is the weather on mars", Expect::NoMatch),
];

/// What a reply comes to, whichever surface it came through.
#[derive(Debug, PartialEq)]
enum Resolution {
    Answered(String),
    Listed(Vec<String>),
    NoMatch,
}

struct Labels(HashMap<String, &'static str>);

impl Labels {
    fn of(&self, id: &str) -> TestResult<String> {
        self.0
            .get(id)
            .map(|label| (*label).to_owned())
            .ok_or_else(|| {
                format!("a reply names a decision the ledger was not seeded with: {id}").into()
            })
    }

    /// Reads a reply's envelope the way a caller does: a candidate list, no match, or the
    /// decision it is about (`root` for `why`, `decision_id` for `verify`).
    fn resolution(&self, envelope: &Value) -> TestResult<Resolution> {
        let data = &envelope["data"];
        match data["outcome"].as_str() {
            Some("not_found") => Ok(Resolution::NoMatch),
            Some("ambiguous") => {
                let candidates = data["candidates"]
                    .as_array()
                    .ok_or("a list has candidates")?;
                let ids = candidates
                    .iter()
                    .map(|candidate| {
                        let id = candidate["decision_id"]
                            .as_str()
                            .ok_or("a candidate has an id")?;
                        self.of(id)
                    })
                    .collect::<TestResult<Vec<String>>>()?;
                Ok(Resolution::Listed(ids))
            }
            _ => {
                let id = data["root"]["id"]
                    .as_str()
                    .or_else(|| data["decision_id"].as_str())
                    .ok_or_else(|| format!("an answer names its decision: {envelope}"))?;
                Ok(Resolution::Answered(self.of(id)?))
            }
        }
    }
}

fn seed(dir: &Path) -> TestResult<Labels> {
    let path = dir.to_str().ok_or("the ledger path is utf-8")?;
    let mut labels = HashMap::new();
    for (label, title, topics, rationale) in DECISIONS {
        let id = run(&Cli::parse_from([
            "loomtracer",
            "--actor",
            "agent:test:parity",
            "--loomtracer-dir",
            path,
            "emit",
            "decision.proposed",
            "--title",
            title,
            "--rationale",
            rationale,
            "--topic-keys",
            topics,
            "--options",
            "A,B",
            "--chose",
            "A",
        ]))?;
        labels.insert(id.trim().to_owned(), *label);
    }
    Ok(Labels(labels))
}

/// An envelope without the one field that differs on every call.
fn without_latency(mut envelope: Value) -> Value {
    if let Some(fields) = envelope.as_object_mut() {
        fields.remove("latency_ms");
    }
    envelope
}

fn via_cli(dir: &Path, verb: &str, question: &str) -> TestResult<Value> {
    let path = dir.to_str().ok_or("the ledger path is utf-8")?;
    let output = run(&Cli::parse_from([
        "loomtracer",
        "--loomtracer-dir",
        path,
        "query",
        verb,
        question,
    ]))?;
    Ok(serde_json::from_str(&output)?)
}

/// The plugin's script, which finds the binary and the ledger the way a user's session does.
fn via_plugin(dir: &Path, verb: &str, question: &str) -> TestResult<Value> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let output = Command::new(root.join(format!("plugins/loomtracer-context/scripts/{verb}.sh")))
        .arg(question)
        .current_dir(root)
        .env("LOOMTRACER_CAPTURE_BIN", env!("CARGO_BIN_EXE_loomtracer"))
        .env("LOOMTRACER_DIR", dir)
        .env("CLAUDE_PROJECT_DIR", root)
        .output()?;
    if !output.status.success() {
        return Err(format!(
            "{verb}.sh failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }
    Ok(serde_json::from_slice(&output.stdout)?)
}

fn app(loomtracer_dir: &Path) -> axum::Router {
    let config = loomtracer::api::ApiConfig {
        loomtracer_dir: loomtracer_dir.to_path_buf(),
        bind: std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
        port: 0,
        allow_unauthenticated_remote: false,
        api_key: None,
        database_url: None,
        admin_key: None,
        workos_domain: None,
        workos_issuer: None,
        workos_jwks_url: None,
        workos_audience: None,
        spa_dir: None,
        cors_origins: vec![],
        slack_client_id: None,
        slack_client_secret: None,
        slack_signing_secret: None,
        slack_api_base_url: None,
    };
    loomtracer::api::create_router(&config)
}

/// `text` as a query-string value.
fn percent_encoded(text: &str) -> String {
    text.bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                char::from(byte).to_string()
            }
            other => format!("%{other:02X}"),
        })
        .collect()
}

async fn via_http(dir: &Path, verb: &str, question: &str) -> TestResult<(StatusCode, Value)> {
    let request = Request::builder()
        .method("GET")
        .uri(format!(
            "/v1/decisions/{verb}?description={}",
            percent_encoded(question)
        ))
        .header("x-loomtracer-actor", "agent:test:parity")
        .body(Body::empty())?;
    let response = app(dir).oneshot(request).await?;
    let status = response.status();
    let bytes = response.into_body().collect().await?.to_bytes();
    Ok((status, serde_json::from_slice(&bytes)?))
}

/// The MCP server, spoken to over its stdio the way a client does.
struct McpServer {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
}

impl McpServer {
    fn start(dir: &Path) -> TestResult<Self> {
        let mut child = Command::new(env!("CARGO_BIN_EXE_loomtracer"))
            .arg("--loomtracer-dir")
            .arg(dir)
            .arg("mcp")
            .arg("--session-id")
            .arg("read-surface-parity")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let stdin = child.stdin.take().ok_or("the server has a stdin")?;
        let stdout = BufReader::new(child.stdout.take().ok_or("the server has a stdout")?);
        let mut server = Self {
            child,
            stdin,
            stdout,
            next_id: 0,
        };
        server.request("initialize", None)?;
        Ok(server)
    }

    fn request(&mut self, method: &str, params: Option<Value>) -> TestResult<Value> {
        self.next_id += 1;
        let mut request = json!({"jsonrpc": "2.0", "id": self.next_id, "method": method});
        if let Some(params) = params {
            request["params"] = params;
        }
        writeln!(self.stdin, "{request}")?;
        self.stdin.flush()?;
        let mut line = String::new();
        self.stdout.read_line(&mut line)?;
        Ok(serde_json::from_str(line.trim())?)
    }

    /// The reply of a read tool: the same envelope the CLI prints.
    fn call(&mut self, tool: &str, description: &str) -> TestResult<Value> {
        let reply = self.request(
            "tools/call",
            Some(json!({"name": tool, "arguments": {"description": description}})),
        )?;
        if reply["result"]["isError"] == Value::Bool(true) {
            return Err(format!("{tool} failed: {reply}").into());
        }
        Ok(reply["result"]["structuredContent"].clone())
    }
}

impl Drop for McpServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn mcp_tool(verb: &str) -> &'static str {
    if verb == "why" {
        "get_decision_neighborhood"
    } else {
        "get_decision_outcome"
    }
}

fn check(expected: &Expect, got: &Resolution) -> bool {
    match (expected, got) {
        (Expect::Answers(label), Resolution::Answered(answered)) => answered == label,
        (Expect::ListsFirst(label), Resolution::Listed(ids)) => {
            ids.first() == Some(&(*label).to_owned())
        }
        (Expect::Lists, Resolution::Listed(ids)) => !ids.is_empty(),
        (Expect::Leads(label), Resolution::Answered(answered)) => answered == label,
        (Expect::Leads(label), Resolution::Listed(ids)) => {
            ids.first() == Some(&(*label).to_owned())
        }
        (Expect::NoMatch, Resolution::NoMatch) => true,
        _ => false,
    }
}

#[tokio::test]
async fn why_and_verify_answer_the_same_through_the_cli_the_plugin_http_and_mcp() -> TestResult<()>
{
    let dir = unique_temp_dir("read-surface-parity");
    let labels = seed(&dir)?;
    let mut mcp = McpServer::start(&dir)?;

    for (verb, question, expected) in CASES {
        let cli = without_latency(via_cli(&dir, verb, question)?);
        let plugin = without_latency(via_plugin(&dir, verb, question)?);
        let from_mcp = without_latency(mcp.call(mcp_tool(verb), question)?);
        let (status, http) = via_http(&dir, verb, question).await?;

        let resolution = labels.resolution(&cli)?;
        assert!(
            check(expected, &resolution),
            "{verb} {question:?}: expected {expected:?}, the CLI came to {resolution:?}: {cli}"
        );

        // hivemind-bbnw.2: an answer carries the quality profile and the days it has held, a list
        // and a miss carry neither. Every surface below is compared with this envelope whole, so
        // the profile is the same on all of them.
        let brief = brief_of(verb, &cli);
        match &resolution {
            Resolution::Answered(_) => {
                assert_seven_dimensions(&brief["quality"], &format!("{verb} {question:?}"));
                assert!(
                    brief["still_holds"]["days_held"].is_u64(),
                    "{verb} {question:?}: the answer says how many days it has held: {cli}"
                );
            }
            Resolution::Listed(_) | Resolution::NoMatch => {
                assert!(
                    brief.get("quality").is_none(),
                    "{verb} {question:?}: only an answer carries a profile: {cli}"
                );
            }
        }

        // The plugin runs the CLI, so its reply is the CLI's, and MCP puts the same envelope in
        // its structured content.
        assert_eq!(
            plugin, cli,
            "{verb} {question:?}: the plugin's reply differs"
        );
        assert_eq!(from_mcp, cli, "{verb} {question:?}: the MCP reply differs");

        // A miss is an answer, so HTTP gives the CLI's `not_found` outcome as a 200 like the rest.
        assert_eq!(status, StatusCode::OK, "{verb} {question:?}: {http}");
        assert_eq!(
            without_latency(http),
            cli,
            "{verb} {question:?}: the HTTP reply differs"
        );
    }

    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}

/// A profile names each of the seven dimensions, as assessed (a level and its reasons) or as not
/// assessed (and why), and carries no composite.
fn assert_seven_dimensions(quality: &Value, what: &str) {
    for dimension in [
        "framing",
        "alternatives",
        "information",
        "reasoning",
        "values_tradeoffs",
        "bias_exposure",
        "calibration",
    ] {
        let entry = &quality[dimension];
        match entry["status"].as_str() {
            Some("assessed") => assert!(
                entry["level"].is_string() && entry["reasons"].is_array(),
                "{what}: {dimension} is assessed with a level and reasons: {entry}"
            ),
            Some("not_assessed") => assert!(
                entry["why"].is_string() && entry.get("level").is_none(),
                "{what}: {dimension} is not assessed and says why: {entry}"
            ),
            other => panic!("{what}: {dimension} has status {other:?}: {quality}"),
        }
    }
    for composite in ["score", "tier", "grade"] {
        assert!(
            quality.get(composite).is_none(),
            "{what}: the profile has no {composite}: {quality}"
        );
    }
}

/// The labels listed under `key` of a brief, sorted (the order options come back in is not part
/// of the answer).
fn listed_options(brief: &Value, key: &str) -> Vec<String> {
    let mut labels: Vec<String> = brief[key]
        .as_array()
        .map(|options| {
            options
                .iter()
                .filter_map(|option| option["label"].as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    labels.sort();
    labels
}

/// The brief inside a reply: `why` nests it under `root`, `verify` is the brief itself.
fn brief_of<'a>(verb: &str, envelope: &'a Value) -> &'a Value {
    if verb == "why" {
        &envelope["data"]["root"]
    } else {
        &envelope["data"]
    }
}

/// An option is rejected only against a choice (hivemind-d6ar). A decision captured with its
/// options and no choice lists them as open on every surface, and a decision with a choice still
/// lists the options it turned down.
#[tokio::test]
async fn a_decision_with_no_choice_lists_its_options_as_open_through_every_surface(
) -> TestResult<()> {
    let dir = unique_temp_dir("read-surface-parity-open-options");
    let path = dir.to_str().ok_or("the ledger path is utf-8")?;
    for (title, rationale, topics, options, chose) in [
        (
            "Checker report cadence: daily or weekly",
            "How often should the checker report? Nobody has decided yet, the mayor will pick",
            "reports",
            "Daily,Weekly",
            None,
        ),
        (
            "Ledger storage backend is shared Postgres",
            "One shared ledger keeps every reader consistent without per-host sync",
            "storage",
            "Shared Postgres,Per-host SQLite",
            Some("Shared Postgres"),
        ),
    ] {
        let mut args = vec![
            "loomtracer",
            "--actor",
            "human:alice",
            "--loomtracer-dir",
            path,
            "emit",
            "decision.proposed",
            "--title",
            title,
            "--rationale",
            rationale,
            "--topic-keys",
            topics,
            "--options",
            options,
        ];
        if let Some(chose) = chose {
            args.extend(["--chose", chose]);
        }
        run(&Cli::parse_from(args))?;
    }
    let mut mcp = McpServer::start(&dir)?;

    for (verb, question, open, rejected, chosen) in [
        (
            "why",
            "checker report cadence",
            vec!["Daily", "Weekly"],
            vec![],
            None,
        ),
        (
            "verify",
            "checker report cadence",
            vec!["Daily", "Weekly"],
            vec![],
            None,
        ),
        (
            "why",
            "ledger storage backend",
            vec![],
            vec!["Per-host SQLite"],
            Some("Shared Postgres"),
        ),
        (
            "verify",
            "ledger storage backend",
            vec![],
            vec!["Per-host SQLite"],
            Some("Shared Postgres"),
        ),
    ] {
        let cli = without_latency(via_cli(&dir, verb, question)?);
        let plugin = without_latency(via_plugin(&dir, verb, question)?);
        let from_mcp = without_latency(mcp.call(mcp_tool(verb), question)?);
        let (status, http) = via_http(&dir, verb, question).await?;
        assert_eq!(status, StatusCode::OK, "{verb} {question:?}: {http}");

        let brief = brief_of(verb, &cli);
        assert_eq!(
            listed_options(brief, "open_options"),
            open,
            "{verb} {question:?}: {cli}"
        );
        assert_eq!(
            listed_options(brief, "rejected_options"),
            rejected,
            "{verb} {question:?}: {cli}"
        );
        assert_eq!(
            brief["chosen_option"]["label"].as_str(),
            chosen,
            "{verb} {question:?}: {cli}"
        );
        if chosen.is_some() {
            assert!(
                brief.get("open_options").is_none(),
                "a decision with a choice has no open options: {cli}"
            );
        }

        assert_eq!(
            plugin, cli,
            "{verb} {question:?}: the plugin's reply differs"
        );
        assert_eq!(from_mcp, cli, "{verb} {question:?}: the MCP reply differs");
        assert_eq!(
            without_latency(http),
            cli,
            "{verb} {question:?}: the HTTP reply differs"
        );
    }

    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}

/// hivemind-erlbl: a list of candidates says what became of each one. A decision that was
/// replaced is listed as replaced and names its replacement, through every surface, in the JSON
/// and in `--summary`; before, only the one-call answer and `recall` said so. A record replaced by
/// one decision is folded into it (hivemind-am6rp, see the test below), so this one is replaced
/// by two: a chain that branches has no single decision to fold into.
#[tokio::test]
async fn a_listed_decision_says_its_status_and_a_replaced_one_names_what_replaced_it_through_every_surface(
) -> TestResult<()> {
    let dir = unique_temp_dir("read-surface-parity-listed-status");
    let path = dir.to_str().ok_or("the ledger path is utf-8")?;
    let propose =
        |title: &str, rationale: &str| propose_decision(path, title, rationale, "pricing");
    let replaced = propose(
        "Pricing is chosen after the testers",
        "The paid shape waits for what the first testers say",
    )?;
    let replacement = propose(
        "Pricing is chosen before the testers",
        "The waitlist page quotes a real price, so the paid shape is chosen first",
    )?;
    let other_replacement = propose(
        "Pricing is chosen when the waitlist opens",
        "The waitlist price question settles the paid shape, whatever the testers say",
    )?;
    let untouched = propose(
        "Pricing is chosen once the trial ends",
        "Nothing is charged during the own-use trial",
    )?;
    for new in [&replacement, &other_replacement] {
        run(&Cli::parse_from([
            "loomtracer",
            "--actor",
            "human:alice",
            "--loomtracer-dir",
            path,
            "emit",
            "decision.superseded",
            "--old",
            &replaced,
            "--new",
            new,
        ]))?;
    }
    let mut mcp = McpServer::start(&dir)?;

    for verb in ["why", "verify"] {
        let question = "pricing chosen";
        let cli = without_latency(via_cli(&dir, verb, question)?);
        let plugin = without_latency(via_plugin(&dir, verb, question)?);
        let from_mcp = without_latency(mcp.call(mcp_tool(verb), question)?);
        let (status, http) = via_http(&dir, verb, question).await?;

        assert_eq!(cli["data"]["outcome"], "ambiguous", "{verb}: {cli}");
        let candidates = cli["data"]["candidates"]
            .as_array()
            .ok_or("a list has candidates")?;
        assert_eq!(candidates.len(), 4, "{verb}: {cli}");
        let candidate = |id: &str| -> TestResult<&Value> {
            candidates
                .iter()
                .find(|candidate| candidate["decision_id"] == id)
                .ok_or_else(|| format!("{verb}: {id} is not listed: {cli}").into())
        };

        let old = candidate(&replaced)?;
        assert_eq!(old["status"], "superseded", "{verb}: {cli}");
        let mut named: Vec<&str> = old["superseded_by"]
            .as_array()
            .ok_or("a replaced decision names what replaced it")?
            .iter()
            .filter_map(Value::as_str)
            .collect();
        named.sort_unstable();
        let mut expected = vec![replacement.as_str(), other_replacement.as_str()];
        expected.sort_unstable();
        assert_eq!(named, expected, "{verb}: {cli}");
        for standing in [&replacement, &other_replacement, &untouched] {
            let candidate = candidate(standing)?;
            assert_eq!(candidate["status"], "proposed", "{verb}: {cli}");
            assert!(candidate.get("superseded_by").is_none(), "{verb}: {cli}");
            assert!(candidate.get("resolved_via").is_none(), "{verb}: {cli}");
        }

        assert_eq!(plugin, cli, "{verb}: the plugin's reply differs");
        assert_eq!(from_mcp, cli, "{verb}: the MCP reply differs");
        assert_eq!(status, StatusCode::OK, "{verb}: {http}");
        assert_eq!(without_latency(http), cli, "{verb}: the HTTP reply differs");

        let summary = run(&Cli::parse_from([
            "loomtracer",
            "--loomtracer-dir",
            path,
            "query",
            verb,
            question,
            "--summary",
        ]))?;
        let line = summary
            .lines()
            .find(|line| line.contains(&replaced))
            .ok_or_else(|| format!("{verb}: {replaced} is not listed: {summary}"))?;
        assert!(
            line.contains("status: superseded by ")
                && line.contains(&replacement)
                && line.contains(&other_replacement),
            "{verb}: {line}"
        );
        let line = summary
            .lines()
            .find(|line| line.contains(&untouched))
            .ok_or_else(|| format!("{verb}: {untouched} is not listed: {summary}"))?;
        assert!(line.ends_with("status: proposed"), "{verb}: {line}");
    }

    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}

/// hivemind-n217f: `chain` for a decision that replaced several records answers, through every
/// surface, with the line to that decision and each replaced record by title. Before, it failed
/// with `supersession chain branched at <id>: N candidates`, naming neither the chain nor the
/// records.
#[tokio::test]
async fn chain_of_a_decision_that_replaced_several_records_names_each_through_every_surface(
) -> TestResult<()> {
    let dir = unique_temp_dir("read-surface-parity-chain-merge");
    let path = dir.to_str().ok_or("the ledger path is utf-8")?;
    let waits_for_beta = propose_decision(
        path,
        "Launch waits for the beta",
        "Nothing is announced before the beta has run",
        "launch",
    )?;
    let waits_for_pricing = propose_decision(
        path,
        "Launch waits for the pricing page",
        "Nobody signs up before the price is public",
        "launch",
    )?;
    let merged = propose_decision(
        path,
        "Launch happens before the beta",
        "The announcement brings the first beta testers",
        "launch",
    )?;
    for old in [&waits_for_beta, &waits_for_pricing] {
        run(&Cli::parse_from([
            "loomtracer",
            "--actor",
            "human:alice",
            "--loomtracer-dir",
            path,
            "emit",
            "decision.superseded",
            "--old",
            old,
            "--new",
            &merged,
        ]))?;
    }
    let mut mcp = McpServer::start(&dir)?;
    let question = "launch";

    let cli = without_latency(via_cli(&dir, "chain", question)?);
    let data = &cli["data"];
    assert_eq!(data["decision_ids"], json!([merged]), "{cli}");
    assert_eq!(data["input_index"], 0, "{cli}");
    let forks = data["forks"]
        .as_array()
        .ok_or_else(|| format!("the answer names the records it replaced: {cli}"))?;
    assert_eq!(forks.len(), 1, "{cli}");
    assert_eq!(forks[0]["at"], merged.as_str(), "{cli}");
    assert_eq!(forks[0]["side"], "older", "{cli}");
    let mut named: Vec<(&str, &str)> = forks[0]["candidates"]
        .as_array()
        .ok_or("a fork has candidates")?
        .iter()
        .map(|candidate| {
            Ok((
                candidate["decision_id"]
                    .as_str()
                    .ok_or("a candidate has an id")?,
                candidate["title"]
                    .as_str()
                    .ok_or("a candidate has a title")?,
            ))
        })
        .collect::<TestResult<_>>()?;
    named.sort_unstable();
    let mut expected = vec![
        (waits_for_beta.as_str(), "Launch waits for the beta"),
        (
            waits_for_pricing.as_str(),
            "Launch waits for the pricing page",
        ),
    ];
    expected.sort_unstable();
    assert_eq!(named, expected, "{cli}");

    let from_mcp = without_latency(mcp.call("get_supersession_chain", question)?);
    assert_eq!(from_mcp, cli, "the MCP reply differs");

    // Named by id, as a follow-up on the id the answer gave.
    let by_id = without_latency(serde_json::from_str(&run(&Cli::parse_from([
        "loomtracer",
        "--loomtracer-dir",
        path,
        "query",
        "chain",
        "--id",
        &merged,
    ]))?)?);
    assert_eq!(by_id["data"], cli["data"], "chain --id differs");
    let response = app(&dir)
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/v1/decisions/{merged}/supersession-chain"))
                .header("x-loomtracer-actor", "agent:test:parity")
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await?.to_bytes();
    let http: Value = serde_json::from_slice(&bytes)?;
    assert_eq!(http["data"], cli["data"], "the HTTP reply differs");

    let summary = run(&Cli::parse_from([
        "loomtracer",
        "--loomtracer-dir",
        path,
        "query",
        "chain",
        question,
        "--summary",
    ]))?;
    assert!(
        summary
            .lines()
            .any(|line| line == format!("BRANCHED: {merged} supersedes 2 decisions")),
        "{summary}"
    );
    for (id, title) in &expected {
        assert!(
            summary
                .lines()
                .any(|line| line == format!("  {id}\t{title}")),
            "no line names {id} by title: {summary}"
        );
    }

    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}

/// One decision proposed on the ledger at `path`, its id.
fn propose_decision(path: &str, title: &str, rationale: &str, topic: &str) -> TestResult<String> {
    let id = run(&Cli::parse_from([
        "loomtracer",
        "--actor",
        "human:alice",
        "--loomtracer-dir",
        path,
        "emit",
        "decision.proposed",
        "--title",
        title,
        "--rationale",
        rationale,
        "--topic-keys",
        topic,
        "--options",
        "Before,After",
    ]))?;
    Ok(id.trim().to_owned())
}

/// hivemind-am6rp: a question that matched a record a later decision replaced is answered with
/// the decision that stands, and the answer names the record, through every surface. Before, the
/// record and its replacement came back as two rivals (`outcome: ambiguous`).
#[tokio::test]
async fn a_replaced_record_answers_with_the_decision_that_replaced_it_through_every_surface(
) -> TestResult<()> {
    let dir = unique_temp_dir("read-surface-parity-replaced-record");
    let path = dir.to_str().ok_or("the ledger path is utf-8")?;
    let replaced = propose_decision(
        path,
        "Launch waits for the beta",
        "Nothing is announced before the beta has run",
        "launch",
    )?;
    let replacement = propose_decision(
        path,
        "Launch happens before the beta",
        "The announcement brings the first beta testers",
        "launch",
    )?;
    run(&Cli::parse_from([
        "loomtracer",
        "--actor",
        "human:alice",
        "--loomtracer-dir",
        path,
        "emit",
        "decision.superseded",
        "--old",
        &replaced,
        "--new",
        &replacement,
    ]))?;
    let mut mcp = McpServer::start(&dir)?;

    for verb in ["why", "verify"] {
        let question = "launch beta";
        let cli = without_latency(via_cli(&dir, verb, question)?);
        let plugin = without_latency(via_plugin(&dir, verb, question)?);
        let from_mcp = without_latency(mcp.call(mcp_tool(verb), question)?);
        let (status, http) = via_http(&dir, verb, question).await?;

        assert_ne!(cli["data"]["outcome"], "ambiguous", "{verb}: {cli}");
        let answered = cli["data"]["root"]["id"]
            .as_str()
            .or_else(|| cli["data"]["decision_id"].as_str())
            .ok_or_else(|| format!("{verb}: an answer names its decision: {cli}"))?;
        assert_eq!(answered, replacement, "{verb}: {cli}");
        let via = cli["resolved_via"]
            .as_array()
            .ok_or_else(|| format!("{verb}: the answer names the record it came through: {cli}"))?;
        assert_eq!(via.len(), 1, "{verb}: {cli}");
        assert_eq!(via[0]["decision_id"], replaced.as_str(), "{verb}: {cli}");
        assert_eq!(
            via[0]["title"], "Launch waits for the beta",
            "{verb}: {cli}"
        );

        assert_eq!(plugin, cli, "{verb}: the plugin's reply differs");
        assert_eq!(from_mcp, cli, "{verb}: the MCP reply differs");
        assert_eq!(status, StatusCode::OK, "{verb}: {http}");
        assert_eq!(without_latency(http), cli, "{verb}: the HTTP reply differs");

        let summary = run(&Cli::parse_from([
            "loomtracer",
            "--loomtracer-dir",
            path,
            "query",
            verb,
            question,
            "--summary",
        ]))?;
        let line = summary
            .lines()
            .find(|line| line.starts_with("resolved via:"))
            .ok_or_else(|| format!("{verb}: no `resolved via:` line: {summary}"))?;
        assert!(line.contains(&replaced), "{verb}: {line}");
    }

    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}

/// hivemind-zdsh.25: a question that matched a decision and a decision that rests on it is
/// answered with the decision it rests on, and the answer names the other as its consequence,
/// through every surface. Before, the two came back as rivals (`outcome: ambiguous`).
#[tokio::test]
async fn a_decision_that_rests_on_the_answer_is_named_as_its_consequence_through_every_surface(
) -> TestResult<()> {
    let dir = unique_temp_dir("read-surface-parity-consequence");
    let path = dir.to_str().ok_or("the ledger path is utf-8")?;
    let root = propose_decision(
        path,
        "Split the frontend into its own repository",
        "The web UI ships on its own cadence",
        "frontend",
    )?;
    let consequence = propose_decision(
        path,
        "Remove the frontend from the main repository now that the split is decided",
        "Nothing else needs the web directory",
        "frontend",
    )?;
    run(&Cli::parse_from([
        "loomtracer",
        "--actor",
        "human:alice",
        "--loomtracer-dir",
        path,
        "emit",
        "relation.added",
        "--kind",
        "follows-from",
        "--from",
        &consequence,
        "--to",
        &root,
    ]))?;
    let mut mcp = McpServer::start(&dir)?;

    for verb in ["why", "verify"] {
        let question = "frontend repository split";
        let cli = without_latency(via_cli(&dir, verb, question)?);
        let plugin = without_latency(via_plugin(&dir, verb, question)?);
        let from_mcp = without_latency(mcp.call(mcp_tool(verb), question)?);
        let (status, http) = via_http(&dir, verb, question).await?;

        assert_ne!(cli["data"]["outcome"], "ambiguous", "{verb}: {cli}");
        let answered = cli["data"]["root"]["id"]
            .as_str()
            .or_else(|| cli["data"]["decision_id"].as_str())
            .ok_or_else(|| format!("{verb}: an answer names its decision: {cli}"))?;
        assert_eq!(answered, root, "{verb}: {cli}");
        let rest_on_it = cli["consequences"]
            .as_array()
            .ok_or_else(|| format!("{verb}: the answer names what rests on it: {cli}"))?;
        assert_eq!(rest_on_it.len(), 1, "{verb}: {cli}");
        assert_eq!(
            rest_on_it[0]["decision_id"],
            consequence.as_str(),
            "{verb}: {cli}"
        );
        assert_eq!(
            rest_on_it[0]["title"],
            "Remove the frontend from the main repository now that the split is decided",
            "{verb}: {cli}"
        );

        assert_eq!(plugin, cli, "{verb}: the plugin's reply differs");
        assert_eq!(from_mcp, cli, "{verb}: the MCP reply differs");
        assert_eq!(status, StatusCode::OK, "{verb}: {http}");
        assert_eq!(without_latency(http), cli, "{verb}: the HTTP reply differs");

        let summary = run(&Cli::parse_from([
            "loomtracer",
            "--loomtracer-dir",
            path,
            "query",
            verb,
            question,
            "--summary",
        ]))?;
        let line = summary
            .lines()
            .find(|line| line.starts_with("consequences:"))
            .ok_or_else(|| format!("{verb}: no `consequences:` line: {summary}"))?;
        assert!(line.contains(&consequence), "{verb}: {line}");
    }

    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}

/// hivemind-701nb: a question that one decision answers alone while another holds every word too
/// is answered with the first, and the answer names the other and why it ranked lower, through
/// every surface. Before, the other was shown nowhere.
#[tokio::test]
async fn an_answer_names_the_full_match_it_out_ranked_through_every_surface() -> TestResult<()> {
    let dir = unique_temp_dir("read-surface-parity-also-matches");
    let path = dir.to_str().ok_or("the ledger path is utf-8")?;
    let leader = propose_decision(
        path,
        "Split the frontend into its own repository",
        "The web UI ships on its own cadence",
        "frontend",
    )?;
    let other = propose_decision(
        path,
        "Keep a read-only mirror of the code on a second host",
        "The mirror carries the frontend repository as it was before the split",
        "mirror",
    )?;
    let mut mcp = McpServer::start(&dir)?;

    for verb in ["why", "verify"] {
        let question = "frontend repository split";
        let cli = without_latency(via_cli(&dir, verb, question)?);
        let plugin = without_latency(via_plugin(&dir, verb, question)?);
        let from_mcp = without_latency(mcp.call(mcp_tool(verb), question)?);
        let (status, http) = via_http(&dir, verb, question).await?;

        assert_ne!(cli["data"]["outcome"], "ambiguous", "{verb}: {cli}");
        let answered = cli["data"]["root"]["id"]
            .as_str()
            .or_else(|| cli["data"]["decision_id"].as_str())
            .ok_or_else(|| format!("{verb}: an answer names its decision: {cli}"))?;
        assert_eq!(answered, leader, "{verb}: {cli}");
        let named = cli["also_matches"]
            .as_array()
            .ok_or_else(|| format!("{verb}: the answer names what it out-ranked: {cli}"))?;
        assert_eq!(named.len(), 1, "{verb}: {cli}");
        assert_eq!(named[0]["decision_id"], other.as_str(), "{verb}: {cli}");
        assert_eq!(
            named[0]["title"], "Keep a read-only mirror of the code on a second host",
            "{verb}: {cli}"
        );
        assert_eq!(
            named[0]["ranked_lower_because"],
            "it says less of the question in its title, topic keys and recorded question",
            "{verb}: {cli}"
        );

        assert_eq!(plugin, cli, "{verb}: the plugin's reply differs");
        assert_eq!(from_mcp, cli, "{verb}: the MCP reply differs");
        assert_eq!(status, StatusCode::OK, "{verb}: {http}");
        assert_eq!(without_latency(http), cli, "{verb}: the HTTP reply differs");

        let summary = run(&Cli::parse_from([
            "loomtracer",
            "--loomtracer-dir",
            path,
            "query",
            verb,
            question,
            "--summary",
        ]))?;
        let line = summary
            .lines()
            .find(|line| line.starts_with("also matches:"))
            .ok_or_else(|| format!("{verb}: no `also matches:` line: {summary}"))?;
        assert!(line.contains(&other), "{verb}: {line}");
        assert_eq!(
            summary
                .lines()
                .filter(|line| line.starts_with("also matches:"))
                .count(),
            1,
            "{verb}: one line names them all: {summary}"
        );
    }

    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}
