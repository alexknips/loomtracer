//! Regression test for hivemind-odmnc: `classify-queue submit` for a batch id the ledger never
//! received must refuse, name the id and append nothing. It used to record an
//! `ingest.batch_classified` event that covered nothing and exit 0, which left the real batches in
//! the queue and read like a success. A worker takes its ids from `classify-queue list`, so an id
//! the ledger holds no received batch for is a typo or another ledger's batch. The same rule as
//! acting on a decision that does not exist (hivemind-5t1uk). A batch that is already classified
//! was received, so sending it again is accepted as before.
//! This spawns the real binary, as an agent does.
//!
//! hivemind-d6tya: over `HIVEMIND_API_URL` the same refusal arrives as the server's 404, and the
//! CLI used to exit 2 as "invalid command line input" for it. The help and both docs say exit 3,
//! and an agent that branches on the code must see one answer whether it asked a ledger or a
//! cell.
//!
//! hivemind-vf0iz: the server answers a token that maps to a tenant it does not hold with its own
//! 404 `unknown_tenant`, and the CLI reads it as the local ledger's storage refusal, exit 4. It
//! must not read as the refused batch id above: every call fails for that one reason.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use hivemind::commands::{CommandContext, Commands};
use hivemind::events::{EventProvenance, TenantId};
use hivemind::ledger::{EventLedger, SqliteEventLedger, SqliteUserStore};

fn unique_dir(label: &str) -> PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push(format!(
        "hivemind-classify-queue-unknown-batch-{label}-{}",
        uuid::Uuid::new_v4()
    ));
    dir
}

/// `hivemind --hivemind-dir <dir> <args>` with no cell URL, no shared database, no
/// `HIVEMIND_DIR` and the default daily cap, unless `envs` says otherwise.
fn hivemind_with(dir: &Path, envs: &[(&str, &str)], args: &[&str]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_hivemind"));
    command
        .env_remove("HIVEMIND_API_URL")
        .env_remove("HIVEMIND_DATABASE_URL")
        .env_remove("HIVEMIND_DIR")
        .env_remove("HIVEMIND_CLASSIFY_DAILY_CAP");
    for (key, value) in envs {
        command.env(key, value);
    }
    command
        .arg("--hivemind-dir")
        .arg(dir)
        .args(args)
        .stdin(std::process::Stdio::null())
        .output()
        .expect("run hivemind") // ubs:ignore: test-only; panicking is correct in tests
}

fn hivemind(dir: &Path, args: &[&str]) -> Output {
    hivemind_with(dir, &[], args)
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// A ledger under `dir` that has received `batch_ids`, the way `POST /v1/ingest` leaves one.
fn ledger_with_batches(dir: &Path, batch_ids: &[&str]) {
    let ledger = SqliteEventLedger::open(dir).expect("open the ledger"); // ubs:ignore: test-only; panicking is correct in tests
    let commands = Commands::new_with_context(
        &ledger,
        CommandContext::new(TenantId::local(), EventProvenance::agent("agent:test")),
    );
    for batch_id in batch_ids {
        commands
            .record_ingest_batch("agent:test", batch_id, "claude", "session-x", Vec::new())
            .expect("receive the batch"); // ubs:ignore: test-only; panicking is correct in tests
    }
}

/// How many events the ledger under `dir` holds.
fn event_count(dir: &Path) -> usize {
    let ledger = SqliteEventLedger::open(dir).expect("open the ledger"); // ubs:ignore: test-only; panicking is correct in tests
    ledger
        .read_for_tenant(&TenantId::local(), 0, 10_000)
        .expect("read the ledger") // ubs:ignore: test-only; panicking is correct in tests
        .len()
}

fn pending_total(dir: &Path) -> u64 {
    let listed = hivemind(dir, &["classify-queue", "list", "--json", "--limit", "1"]);
    assert!(listed.status.success(), "{}", stderr(&listed));
    let queue: serde_json::Value =
        serde_json::from_str(&stdout(&listed)).expect("the queue is JSON"); // ubs:ignore: test-only; panicking is correct in tests
    queue["pending_total"].as_u64().expect("a pending_total") // ubs:ignore: test-only; panicking is correct in tests
}

fn submit(dir: &Path, batch_ids: &str) -> Output {
    submit_with(dir, &[], batch_ids)
}

fn submit_with(dir: &Path, envs: &[(&str, &str)], batch_ids: &str) -> Output {
    hivemind_with(
        dir,
        envs,
        &[
            "classify-queue",
            "submit",
            "--batch-id",
            batch_ids,
            "--captures",
            "[]",
        ],
    )
}

#[test]
fn a_batch_nobody_received_is_refused_by_name_and_appends_nothing() {
    let dir = unique_dir("unknown");
    ledger_with_batches(&dir, &["session-x:0-1"]);
    let before = event_count(&dir);

    let output = submit(&dir, "typo:0-1");

    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    assert!(
        stderr(&output).contains("ingest batch does not exist: typo:0-1"),
        "the refusal names the id: {}",
        stderr(&output)
    );
    assert!(stdout(&output).is_empty(), "{}", stdout(&output));
    assert_eq!(event_count(&dir), before, "an event was appended");
    assert_eq!(pending_total(&dir), 1, "the real batch is still waiting");
}

#[test]
fn one_unknown_id_among_received_ones_refuses_the_whole_submit() {
    let dir = unique_dir("mixed");
    ledger_with_batches(&dir, &["session-x:0-1", "session-x:1-2"]);
    let before = event_count(&dir);

    let output = submit(&dir, "session-x:0-1,typo:0-1,session-x:1-2");

    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let message = stderr(&output);
    assert!(
        message.contains("ingest batch does not exist: typo:0-1."),
        "only the unknown id is named: {message}"
    );
    assert_eq!(event_count(&dir), before, "an event was appended");
    assert_eq!(
        pending_total(&dir),
        2,
        "both real batches are still waiting"
    );
}

#[test]
fn a_received_batch_is_classified_and_can_be_sent_again() {
    let dir = unique_dir("received");
    ledger_with_batches(&dir, &["session-x:0-1"]);
    let before = event_count(&dir);

    let first = submit(&dir, "session-x:0-1");

    assert!(first.status.success(), "{}", stderr(&first));
    assert_eq!(event_count(&dir), before + 1);
    assert_eq!(pending_total(&dir), 0, "the batch left the queue");

    // Concurrent workers may classify the same batch; the second send is not refused.
    let again = submit(&dir, "session-x:0-1");

    assert!(again.status.success(), "{}", stderr(&again));
    assert_eq!(event_count(&dir), before + 2);
}

#[test]
fn an_unknown_batch_is_not_answered_with_the_daily_cap() {
    let dir = unique_dir("cap");
    ledger_with_batches(&dir, &["session-x:0-1", "session-x:1-2"]);
    let capped = [("HIVEMIND_CLASSIFY_DAILY_CAP", "1")];

    let first = submit_with(&dir, &capped, "session-x:0-1");
    assert!(first.status.success(), "{}", stderr(&first));

    // The cap is spent. A batch that is not waiting is still a bad id, not "wait until tomorrow".
    let unknown = submit_with(&dir, &capped, "typo:0-1");
    assert_eq!(unknown.status.code(), Some(3), "{}", stderr(&unknown));
    assert!(
        stderr(&unknown).contains("ingest batch does not exist: typo:0-1"),
        "{}",
        stderr(&unknown)
    );

    // A real batch past the cap is refused for the cap, and stays pending.
    let over = submit_with(&dir, &capped, "session-x:1-2");
    assert_eq!(over.status.code(), Some(2), "{}", stderr(&over));
    assert!(
        stderr(&over).contains("daily classification cap"),
        "{}",
        stderr(&over)
    );
    assert_eq!(pending_total(&dir), 1);
}

// ---------------------------------------------------------------------------
// Over HIVEMIND_API_URL (hivemind-d6tya)
// ---------------------------------------------------------------------------

const STARTUP_TIMEOUT: Duration = Duration::from_secs(60);

/// tracing colors its fields even on a pipe; drop the SGR escapes so `addr=host:port` reads plain.
fn strip_ansi(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            // Skip `ESC [ ... <final byte>`.
            for c in chars.by_ref() {
                if c.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// A `hivemind serve --port 0` process on a ledger directory, killed when dropped.
struct Served {
    child: Child,
    url: String,
}

impl Served {
    fn start(dir: &Path) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_hivemind"))
            .env_remove("HIVEMIND_API_KEY")
            .env_remove("HIVEMIND_ADMIN_KEY")
            .env_remove("HIVEMIND_DATABASE_URL")
            .env_remove("HIVEMIND_BIND")
            .env_remove("HIVEMIND_PORT")
            .env_remove("WORKOS_DOMAIN")
            .args(["-v", "--hivemind-dir"])
            .arg(dir)
            .args(["serve", "--port", "0"])
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn hivemind serve"); // ubs:ignore: test-only; panicking is correct in tests
        let stderr = child.stderr.take().expect("piped stderr"); // ubs:ignore: test-only; panicking is correct in tests
        let (tx, lines) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                if tx.send(strip_ansi(&line)).is_err() {
                    break;
                }
            }
        });
        // `Served` exists before the wait, so a server that never listens is still killed.
        let mut served = Self {
            child,
            url: String::new(),
        };
        let deadline = Instant::now() + STARTUP_TIMEOUT;
        let mut seen = Vec::new();
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let line = lines.recv_timeout(left).unwrap_or_else(|e| {
                panic!("server never reported listening ({e}); stderr so far: {seen:#?}")
            }); // ubs:ignore: test-only
            if line.contains("HTTP API listening") {
                let addr = line
                    .split("addr=")
                    .nth(1)
                    .and_then(|rest| rest.split_whitespace().next())
                    .unwrap_or_else(|| panic!("no addr= in startup line: {line}")); // ubs:ignore: test-only
                served.url = format!("http://{addr}");
                return served;
            }
            seen.push(line);
        }
    }
}

impl Drop for Served {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// `classify-queue submit` the way an agent on a cell runs it: from a folder of its own that
/// holds no ledger, with only `HIVEMIND_API_URL` pointing at the server.
fn submit_over(url: &str, own_dir: &Path, batch_ids: &str) -> Output {
    submit_with(own_dir, &[("HIVEMIND_API_URL", url)], batch_ids)
}

#[test]
fn over_a_cell_url_a_batch_nobody_received_exits_3_naming_it_and_records_nothing() {
    let served_dir = unique_dir("served-unknown");
    ledger_with_batches(&served_dir, &["session-x:0-1"]);
    let before = event_count(&served_dir);
    let own_dir = unique_dir("own-unknown");
    let served = Served::start(&served_dir);

    let output = submit_over(&served.url, &own_dir, "nope");

    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let message = stderr(&output);
    assert!(
        message.contains("ingest batch does not exist: nope."),
        "the refusal names the id: {message}"
    );
    assert!(
        !message.contains("invalid command line input"),
        "the input was fine, the server answered not found: {message}"
    );
    assert!(stdout(&output).is_empty(), "{}", stdout(&output));
    assert_eq!(event_count(&served_dir), before, "an event was appended");
    assert_eq!(
        pending_total(&served_dir),
        1,
        "the real batch is still waiting"
    );
    assert!(
        !own_dir.exists(),
        "the CLI made a ledger of its own beside the cell"
    );
}

#[test]
fn the_refusal_reads_the_same_over_a_cell_url_as_on_a_local_ledger() {
    let served_dir = unique_dir("served-same");
    ledger_with_batches(&served_dir, &["session-x:0-1"]);
    let local_dir = unique_dir("local-same");
    ledger_with_batches(&local_dir, &["session-x:0-1"]);
    let own_dir = unique_dir("own-same");
    let served = Served::start(&served_dir);

    let over_cell = submit_over(&served.url, &own_dir, "typo:0-1");
    let local = submit(&local_dir, "typo:0-1");

    assert_eq!(over_cell.status.code(), Some(3), "{}", stderr(&over_cell));
    assert_eq!(local.status.code(), Some(3), "{}", stderr(&local));
    assert_eq!(
        stderr(&over_cell),
        stderr(&local),
        "an agent reads one refusal whichever it asked"
    );
}

#[test]
fn over_a_cell_url_one_unknown_id_among_received_ones_refuses_the_whole_submit() {
    let served_dir = unique_dir("served-mixed");
    ledger_with_batches(&served_dir, &["session-x:0-1", "session-x:1-2"]);
    let before = event_count(&served_dir);
    let own_dir = unique_dir("own-mixed");
    let served = Served::start(&served_dir);

    let output = submit_over(
        &served.url,
        &own_dir,
        "session-x:0-1,typo:0-1,session-x:1-2",
    );

    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    assert!(
        stderr(&output).contains("ingest batch does not exist: typo:0-1."),
        "only the unknown id is named: {}",
        stderr(&output)
    );
    assert_eq!(event_count(&served_dir), before, "an event was appended");
    assert_eq!(pending_total(&served_dir), 2);
}

#[test]
fn over_a_cell_url_a_received_batch_is_still_classified() {
    let served_dir = unique_dir("served-received");
    ledger_with_batches(&served_dir, &["session-x:0-1"]);
    let before = event_count(&served_dir);
    let own_dir = unique_dir("own-received");
    let served = Served::start(&served_dir);

    let output = submit_over(&served.url, &own_dir, "session-x:0-1");

    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(event_count(&served_dir), before + 1);
    assert_eq!(pending_total(&served_dir), 0, "the batch left the queue");
}

// ---------------------------------------------------------------------------
// An unknown tenant over HIVEMIND_API_URL (hivemind-vf0iz)
// ---------------------------------------------------------------------------

const GHOST_TENANT: &str = "ghost";

/// A bearer token for `GHOST_TENANT`, a tenant the ledger under `dir` never registered: the shape
/// of an agent whose token maps to a tenant the cell does not hold.
fn token_for_unregistered_tenant(dir: &Path) -> String {
    SqliteUserStore::open(dir)
        .expect("open the user store") // ubs:ignore: test-only; panicking is correct in tests
        .mint_agent_token(GHOST_TENANT, "agent:test:ghost", None)
        .expect("mint a token") // ubs:ignore: test-only; panicking is correct in tests
        .token_secret
}

fn list_over(url: &str, token: &str, own_dir: &Path) -> Output {
    hivemind_with(
        own_dir,
        &[("HIVEMIND_API_URL", url), ("HIVEMIND_API_KEY", token)],
        &["classify-queue", "list"],
    )
}

fn submit_over_as(url: &str, token: &str, own_dir: &Path, batch_ids: &str) -> Output {
    hivemind_with(
        own_dir,
        &[("HIVEMIND_API_URL", url), ("HIVEMIND_API_KEY", token)],
        &[
            "classify-queue",
            "submit",
            "--batch-id",
            batch_ids,
            "--captures",
            "[]",
        ],
    )
}

/// What a local ledger says for the same condition: the tenant is named, not registered.
fn assert_unknown_tenant_refusal(output: &Output) {
    let message = stderr(output);
    assert_eq!(output.status.code(), Some(4), "{message}");
    assert!(
        message.contains("unknown tenant 'ghost'"),
        "the refusal names the tenant: {message}"
    );
    assert!(
        !message.contains("invariant violated"),
        "an unknown tenant is not a refused batch id (exit 3): {message}"
    );
    assert!(
        !message.contains("invalid command line input"),
        "the input was fine: {message}"
    );
    assert!(stdout(output).is_empty(), "{}", stdout(output));
}

#[test]
fn over_a_cell_url_an_unknown_tenant_exits_4_on_list_and_on_submit_and_records_nothing() {
    let served_dir = unique_dir("served-tenant");
    ledger_with_batches(&served_dir, &["session-x:0-1"]);
    let token = token_for_unregistered_tenant(&served_dir);
    let before = event_count(&served_dir);
    let own_dir = unique_dir("own-tenant");
    let served = Served::start(&served_dir);

    assert_unknown_tenant_refusal(&list_over(&served.url, &token, &own_dir));
    // A batch id the cell does hold, so only the tenant can be what is refused.
    assert_unknown_tenant_refusal(&submit_over_as(
        &served.url,
        &token,
        &own_dir,
        "session-x:0-1",
    ));
    // An id nobody received gets the tenant refusal too: the tenant is checked first, and exit 3
    // would tell the agent to skip that one id.
    assert_unknown_tenant_refusal(&submit_over_as(&served.url, &token, &own_dir, "typo:0-1"));

    assert_eq!(event_count(&served_dir), before, "an event was appended");
    assert_eq!(pending_total(&served_dir), 1, "the batch is still waiting");
    assert!(
        !own_dir.exists(),
        "the CLI made a ledger of its own beside the cell"
    );
}

#[test]
fn the_unknown_tenant_refusal_reads_the_same_over_a_cell_url_as_on_a_local_ledger() {
    let served_dir = unique_dir("served-tenant-same");
    ledger_with_batches(&served_dir, &["session-x:0-1"]);
    let token = token_for_unregistered_tenant(&served_dir);
    let local_dir = unique_dir("local-tenant-same");
    ledger_with_batches(&local_dir, &["session-x:0-1"]);
    let own_dir = unique_dir("own-tenant-same");
    let served = Served::start(&served_dir);

    let local_list = hivemind(
        &local_dir,
        &["--tenant", GHOST_TENANT, "classify-queue", "list"],
    );
    let local_submit = hivemind(
        &local_dir,
        &[
            "--tenant",
            GHOST_TENANT,
            "classify-queue",
            "submit",
            "--batch-id",
            "session-x:0-1",
            "--captures",
            "[]",
        ],
    );
    let over_list = list_over(&served.url, &token, &own_dir);
    let over_submit = submit_over_as(&served.url, &token, &own_dir, "session-x:0-1");

    assert_unknown_tenant_refusal(&local_list);
    assert_unknown_tenant_refusal(&local_submit);
    assert_eq!(
        stderr(&over_list),
        stderr(&local_list),
        "an agent reads one refusal whichever it asked: list"
    );
    assert_eq!(
        stderr(&over_submit),
        stderr(&local_submit),
        "an agent reads one refusal whichever it asked: submit"
    );
}

#[test]
fn over_a_cell_url_a_registered_tenants_token_still_gets_exit_3_for_an_unknown_batch() {
    let served_dir = unique_dir("served-registered");
    ledger_with_batches(&served_dir, &["session-x:0-1"]);
    SqliteEventLedger::open(&served_dir)
        .expect("open the ledger") // ubs:ignore: test-only; panicking is correct in tests
        .create_tenant(&TenantId::new("acme").expect("a tenant id")) // ubs:ignore: test-only
        .expect("register the tenant"); // ubs:ignore: test-only; panicking is correct in tests
    let token = SqliteUserStore::open(&served_dir)
        .expect("open the user store") // ubs:ignore: test-only; panicking is correct in tests
        .mint_agent_token("acme", "agent:test:acme", None)
        .expect("mint a token") // ubs:ignore: test-only; panicking is correct in tests
        .token_secret;
    let own_dir = unique_dir("own-registered");
    let served = Served::start(&served_dir);

    let output = submit_over_as(&served.url, &token, &own_dir, "typo:0-1");

    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    assert!(
        stderr(&output).contains("ingest batch does not exist: typo:0-1"),
        "{}",
        stderr(&output)
    );
}

/// A server that is not HiveMind's answers 404 in its own words, not as the ledger's refusal:
/// that stays the generic failure, exit 2, not an invariant violation about a batch.
#[test]
fn a_404_that_is_not_the_servers_not_found_stays_a_failed_request() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind a stand-in server"); // ubs:ignore: test-only; panicking is correct in tests
    let url = format!("http://{}", listener.local_addr().expect("local addr")); // ubs:ignore: test-only
    let answering = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept the request"); // ubs:ignore: test-only
        let mut request = [0_u8; 4096];
        let _ = stream.read(&mut request);
        let body = r#"{"error":{"code":"route_missing","message":"no such route"}}"#;
        let reply = format!(
            "HTTP/1.1 404 Not Found\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(reply.as_bytes()).expect("write the reply"); // ubs:ignore: test-only
    });
    let own_dir = unique_dir("own-proxy");

    let output = submit_over(&url, &own_dir, "nope");
    answering.join().expect("the stand-in server finished"); // ubs:ignore: test-only

    assert_eq!(output.status.code(), Some(2), "{}", stderr(&output));
    assert!(
        stderr(&output).contains("server returned 404 Not Found: no such route"),
        "{}",
        stderr(&output)
    );
}
