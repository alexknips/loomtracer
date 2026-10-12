//! "Which binding decisions have a why still to answer" lists the same decisions through every
//! surface (hivemind-lapcm).
//!
//! One read serves the CLI, the HTTP API, the stdio MCP server and the HTTP MCP endpoint, but each
//! of them builds its own request and its own reply. Nothing but this test stops one from listing
//! a decision another leaves off, dropping `left_out`, or paging differently. The ledger holds one
//! decision of every kind the list has to tell apart: two that rest only on their own capture, one
//! below the decision floor, one recorded before the floor existed, one that follows from an
//! earlier decision, one a person decided, two that share an observation, and one someone
//! attributed more grounding to afterwards.

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use clap::Parser;
use http_body_util::BodyExt as _;
use serde_json::{json, Value};
use tower::ServiceExt as _;

use hivemind::cli::{run, Cli};

#[allow(dead_code)]
#[path = "support/seed_data.rs"]
mod seed_data;

use seed_data::{unique_temp_dir, TestResult};

const RECORDER: &str = "agent:test:open-why";
const PERSON: &str = "human:alex.knips@example.com";

/// Decision ids by label, in the order they were recorded.
struct Seeded {
    ids: Vec<(&'static str, String)>,
}

impl Seeded {
    fn id(&self, label: &str) -> &str {
        self.ids
            .iter()
            .find(|(candidate, _)| *candidate == label)
            .map(|(_, id)| id.as_str())
            .unwrap_or_else(|| panic!("no decision labelled {label}")) // ubs:ignore: test-only; panicking is correct in tests
    }

    fn label(&self, id: &str) -> &str {
        self.ids
            .iter()
            .find(|(_, candidate)| candidate == id)
            .map(|(label, _)| *label)
            .unwrap_or_else(|| {
                panic!("a reply names a decision the ledger was not seeded with: {id}")
            }) // ubs:ignore: test-only; panicking is correct in tests
    }
}

fn cli(dir: &Path, args: &[&str]) -> TestResult<String> {
    let path = dir.to_str().ok_or("the ledger path is utf-8")?;
    let mut argv = vec!["hivemind", "--actor", RECORDER, "--hivemind-dir", path];
    argv.extend_from_slice(args);
    Ok(run(&Cli::parse_from(argv))?.trim().to_owned())
}

/// A `decision.capture` with the answers a caller gives, plus whatever else `extra` adds.
fn capture(dir: &Path, title: &str, floor: &[&str], extra: &[&str]) -> TestResult<String> {
    let mut args = vec![
        "emit",
        "decision.capture",
        "--title",
        title,
        "--rationale",
        "Recorded for the open-why parity check, long enough to read on its own",
        "--topic-keys",
        "open-why",
        "--options",
        "A,B",
        "--chose",
        "A",
    ];
    args.extend_from_slice(floor);
    args.extend_from_slice(extra);
    cli(dir, &args)
}

const BINDING: &[&str] = &[
    "--binds",
    "every script that calls the old name",
    "--cheap-to-undo",
];
const COSTLY: &[&str] = &[
    "--binds-nothing",
    "--undo-cost",
    "a merge cannot be unmerged",
];
const BELOW: &[&str] = &["--binds-nothing", "--cheap-to-undo"];

fn seed(dir: &Path) -> TestResult<Seeded> {
    let mut ids: Vec<(&'static str, String)> = vec![(
        "open",
        capture(
            dir,
            "Rename the check command to verify",
            BINDING,
            &[
                "--rests-on-evidence",
                "14 scripts call it",
                "--evidence-source",
                "grep@abc123",
            ],
        )?,
    )];
    ids.push((
        "open-costly",
        capture(dir, "Merge the release branch tonight", COSTLY, &["--bet"])?,
    ));
    ids.push((
        "below",
        capture(
            dir,
            "Type apple, not Apple",
            BELOW,
            &["--rests-on-evidence", "the shop sells apples"],
        )?,
    ));
    ids.push((
        "old",
        cli(
            dir,
            &[
                "emit",
                "decision.proposed",
                "--title",
                "Recorded before the floor existed",
                "--rationale",
                "Recorded for the open-why parity check, long enough to read on its own",
                "--topic-keys",
                "open-why",
                "--options",
                "A,B",
                "--chose",
                "A",
            ],
        )?,
    ));
    let earlier = ids[0].1.clone();
    ids.push((
        "follower",
        capture(
            dir,
            "Print the new name in every help text",
            BINDING,
            &["--rests-on-decision", &earlier],
        )?,
    ));
    ids.push((
        "by-person",
        capture(
            dir,
            "Alex rules the name stays verify",
            BINDING,
            &[
                "--decided-by",
                PERSON,
                "--rests-on-evidence",
                "Alex said so",
            ],
        )?,
    ));
    ids.push((
        "shared-a",
        capture(
            dir,
            "Cache the parsed config",
            BINDING,
            &["--rests-on-evidence", "parsing was 40ms per call"],
        )?,
    ));
    ids.push((
        "shared-b",
        capture(
            dir,
            "Cache the parsed schema",
            BINDING,
            &["--rests-on-evidence", "schema parsing was 25ms per call"],
        )?,
    ));
    ids.push((
        "regrounded",
        capture(
            dir,
            "Pin the toolchain version",
            BINDING,
            &["--rests-on-evidence", "a build broke on nightly"],
        )?,
    ));

    let seeded = Seeded { ids };
    // `shared-b` also rests on the observation `shared-a` brought: the two are linked.
    let shared_evidence = evidence_id_of(dir, seeded.id("shared-a"))?;
    cli(
        dir,
        &[
            "ground",
            "--id",
            seeded.id("shared-b"),
            "--evidence",
            &shared_evidence,
        ],
    )?;
    // Someone attributes a later review to `regrounded`: it no longer rests only on its capture.
    cli(
        dir,
        &[
            "ground",
            "--id",
            seeded.id("regrounded"),
            "--rests-on-evidence",
            "a reviewer re-ran the build on nightly",
        ],
    )?;
    Ok(seeded)
}

/// The id of the one observation a listed decision rests on.
fn evidence_id_of(dir: &Path, decision_id: &str) -> TestResult<String> {
    let reply: Value = serde_json::from_str(&cli(
        dir,
        &["query", "get_open_why_decisions", "--limit", "100"],
    )?)?;
    let row = reply["data"]["items"]
        .as_array()
        .ok_or("the reply lists items")?
        .iter()
        .find(|row| row["decision_id"] == json!(decision_id))
        .ok_or("the decision is listed before it is regrounded")?;
    Ok(row["rests_on"][0]["id"]
        .as_str()
        .ok_or("it rests on one observation")?
        .to_owned())
}

/// An envelope without the one field that differs on every call.
fn without_latency(mut envelope: Value) -> Value {
    if let Some(fields) = envelope.as_object_mut() {
        fields.remove("latency_ms");
    }
    envelope
}

/// The request as each surface spells it: `limit` and `cursor`.
#[derive(Clone, Debug, Default)]
struct Ask {
    limit: Option<usize>,
    cursor: Option<String>,
}

impl Ask {
    fn cli_args(&self) -> Vec<String> {
        let mut args = Vec::new();
        if let Some(limit) = self.limit {
            args.extend(["--limit".to_owned(), limit.to_string()]);
        }
        if let Some(cursor) = &self.cursor {
            args.extend(["--cursor".to_owned(), cursor.clone()]);
        }
        args
    }

    fn query_string(&self) -> String {
        let mut parts = Vec::new();
        if let Some(limit) = self.limit {
            parts.push(format!("limit={limit}"));
        }
        if let Some(cursor) = &self.cursor {
            parts.push(format!("cursor={}", percent_encoded(cursor)));
        }
        parts.join("&")
    }

    fn tool_arguments(&self) -> Value {
        let mut arguments = json!({});
        if let Some(limit) = self.limit {
            arguments["limit"] = json!(limit);
        }
        if let Some(cursor) = &self.cursor {
            arguments["cursor"] = json!(cursor);
        }
        arguments
    }
}

fn via_cli(dir: &Path, ask: &Ask) -> TestResult<Value> {
    let path = dir.to_str().ok_or("the ledger path is utf-8")?;
    let mut argv = vec![
        "hivemind".to_owned(),
        "--hivemind-dir".to_owned(),
        path.to_owned(),
        "query".to_owned(),
        "get_open_why_decisions".to_owned(),
    ];
    argv.extend(ask.cli_args());
    let output = run(&Cli::parse_from(argv))?;
    Ok(serde_json::from_str(&output)?)
}

fn app(hivemind_dir: &Path) -> axum::Router {
    let config = hivemind::api::ApiConfig {
        hivemind_dir: hivemind_dir.to_path_buf(),
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
    hivemind::api::create_router(&config)
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

async fn via_http(dir: &Path, ask: &Ask) -> TestResult<(StatusCode, Value)> {
    let query = ask.query_string();
    let uri = if query.is_empty() {
        "/v1/attention/open-why".to_owned()
    } else {
        format!("/v1/attention/open-why?{query}")
    };
    let request = Request::builder()
        .method("GET")
        .uri(uri)
        .header("x-hivemind-actor", RECORDER)
        .body(Body::empty())?;
    let response = app(dir).oneshot(request).await?;
    let status = response.status();
    let bytes = response.into_body().collect().await?.to_bytes();
    Ok((status, serde_json::from_slice(&bytes)?))
}

/// The same tool over the HTTP MCP endpoint.
async fn via_http_mcp(dir: &Path, ask: &Ask) -> TestResult<Value> {
    let body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": "get_open_why_decisions",
            "arguments": ask.tool_arguments(),
        },
    });
    let request = Request::builder()
        .method("POST")
        .uri("/mcp")
        .header("content-type", "application/json")
        .header("x-hivemind-actor", RECORDER)
        .body(Body::from(serde_json::to_string(&body)?))?;
    let response = app(dir).oneshot(request).await?;
    let bytes = response.into_body().collect().await?.to_bytes();
    let reply: Value = serde_json::from_slice(&bytes)?;
    if reply["result"]["isError"] == Value::Bool(true) {
        return Err(format!("get_open_why_decisions failed over HTTP MCP: {reply}").into());
    }
    Ok(reply["result"]["structuredContent"].clone())
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
        let mut child = Command::new(env!("CARGO_BIN_EXE_hivemind"))
            .arg("--hivemind-dir")
            .arg(dir)
            .arg("mcp")
            .arg("--session-id")
            .arg("open-why-surface-parity")
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

    /// The reply of the read tool: the same envelope the CLI prints.
    fn open_why(&mut self, ask: &Ask) -> TestResult<Value> {
        let reply = self.request(
            "tools/call",
            Some(json!({
                "name": "get_open_why_decisions",
                "arguments": ask.tool_arguments(),
            })),
        )?;
        if reply["result"]["isError"] == Value::Bool(true) {
            return Err(format!("get_open_why_decisions failed: {reply}").into());
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

/// Every surface's reply to `ask`, which must all be the CLI's.
async fn answer_alike(dir: &Path, mcp: &mut McpServer, ask: &Ask) -> TestResult<Value> {
    let cli = without_latency(via_cli(dir, ask)?);
    let stdio = without_latency(mcp.open_why(ask)?);
    let http_mcp = without_latency(via_http_mcp(dir, ask).await?);
    let (status, http) = via_http(dir, ask).await?;

    assert_eq!(status, StatusCode::OK, "{ask:?}: {http}");
    assert_eq!(stdio, cli, "{ask:?}: the stdio MCP reply differs");
    assert_eq!(http_mcp, cli, "{ask:?}: the HTTP MCP reply differs");
    assert_eq!(
        without_latency(http),
        cli,
        "{ask:?}: the HTTP reply differs"
    );
    Ok(cli)
}

fn labels<'a>(seeded: &'a Seeded, reply: &Value) -> TestResult<Vec<&'a str>> {
    reply["data"]["items"]
        .as_array()
        .ok_or("the reply lists items")?
        .iter()
        .map(|row| -> TestResult<&str> {
            let id = row["decision_id"].as_str().ok_or("a row has an id")?;
            Ok(seeded.label(id))
        })
        .collect()
}

fn row<'a>(seeded: &Seeded, reply: &'a Value, label: &str) -> TestResult<&'a Value> {
    let id = seeded.id(label);
    Ok(reply["data"]["items"]
        .as_array()
        .ok_or("the reply lists items")?
        .iter()
        .find(|row| row["decision_id"] == json!(id))
        .ok_or_else(|| format!("{label} is not listed: {reply}"))?)
}

#[tokio::test]
async fn only_binding_decisions_resting_on_their_own_capture_are_listed_on_every_surface(
) -> TestResult<()> {
    let dir = unique_temp_dir("open-why-surface-parity");
    let seeded = seed(&dir)?;
    let mut mcp = McpServer::start(&dir)?;

    let reply = answer_alike(&dir, &mut mcp, &Ask::default()).await?;

    // Below the floor, never asked, following an earlier decision, decided by a person, sharing an
    // observation and re-grounded are all left off. The two that rest only on their own capture
    // are listed, oldest first.
    assert_eq!(labels(&seeded, &reply)?, ["open", "open-costly"]);
    assert_eq!(reply["data"]["total_matches"], json!(2));
    assert_eq!(reply["truncated"], json!(false));
    assert_eq!(
        reply["data"]["left_out"],
        json!({"below_floor": 1, "floor_not_asked": 1})
    );

    let open = row(&seeded, &reply, "open")?;
    assert_eq!(open["title"], json!("Rename the check command to verify"));
    assert_eq!(open["status"], json!("accepted"));
    assert_eq!(open["floor"], json!("cleared"));
    assert_eq!(
        open["binds"],
        json!({"level": "something", "statement": "every script that calls the old name"})
    );
    assert_eq!(open["undo_cost"], json!({"level": "cheap"}));
    assert_eq!(open["recorded_by"], json!(RECORDER));
    assert_eq!(open["deciders"], json!([{"id": RECORDER, "kind": "agent"}]));
    assert!(open["captured_at"].is_string(), "{open}");
    assert_eq!(open["rests_on"].as_array().map(Vec::len), Some(1));
    assert_eq!(open["rests_on"][0]["kind"], json!("evidence"));
    assert_eq!(open["rests_on"][0]["label"], json!("14 scripts call it"));
    assert_eq!(open["rests_on"][0]["source"], json!("grep@abc123"));

    // Costly to undo clears the floor on its own, and a bet is the capture's own grounding.
    let costly = row(&seeded, &reply, "open-costly")?;
    assert_eq!(costly["floor"], json!("cleared"));
    assert_eq!(costly["binds"], json!({"level": "nothing"}));
    assert_eq!(costly["rests_on"][0]["kind"], json!("bet"));

    // An open state, not a ranking: no score, no tier, no suggested parent.
    let text = reply.to_string();
    for word in [
        "\"score\"",
        "\"rank\"",
        "\"tier\"",
        "\"suggested",
        "\"parent\"",
    ] {
        assert!(!text.contains(word), "{word} in {text}");
    }

    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}

#[tokio::test]
async fn pages_mean_the_same_on_every_surface() -> TestResult<()> {
    let dir = unique_temp_dir("open-why-surface-parity-pages");
    let seeded = seed(&dir)?;
    let mut mcp = McpServer::start(&dir)?;

    let first = answer_alike(
        &dir,
        &mut mcp,
        &Ask {
            limit: Some(1),
            ..Ask::default()
        },
    )
    .await?;
    assert_eq!(labels(&seeded, &first)?, ["open"]);
    assert_eq!(first["truncated"], json!(true));
    assert_eq!(first["data"]["total_matches"], json!(2));
    let cursor = first["data"]["next_cursor"]
        .as_str()
        .ok_or("a truncated page says where to resume")?
        .to_owned();

    let last = answer_alike(
        &dir,
        &mut mcp,
        &Ask {
            limit: Some(1),
            cursor: Some(cursor),
        },
    )
    .await?;
    assert_eq!(labels(&seeded, &last)?, ["open-costly"]);
    assert_eq!(last["truncated"], json!(false));
    assert!(last["data"]["next_cursor"].is_null(), "{last}");

    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}

#[tokio::test]
async fn the_summary_names_who_decided_what_it_rests_on_and_what_was_left_out() -> TestResult<()> {
    let dir = unique_temp_dir("open-why-surface-parity-summary");
    seed(&dir)?;

    let summary = cli(&dir, &["query", "--summary", "get_open_why_decisions"])?;

    let lines: Vec<&str> = summary.lines().collect();
    assert_eq!(lines.len(), 3, "{summary}");
    assert!(lines[0].starts_with("open_why\t"), "{summary}");
    assert!(
        lines[0].contains("Rename the check command to verify"),
        "{summary}"
    );
    assert!(
        lines[0].contains(&format!("decided_by={RECORDER}")),
        "{summary}"
    );
    assert!(
        lines[0].contains("binds: something (every script that calls the old name)"),
        "{summary}"
    );
    assert!(
        lines[0].contains("rests_on=evidence: 14 scripts call it"),
        "{summary}"
    );
    assert_eq!(lines[2], "left_out\tbelow_floor=1\tfloor_not_asked=1");

    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}
