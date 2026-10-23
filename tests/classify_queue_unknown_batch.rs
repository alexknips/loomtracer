//! Regression test for hivemind-odmnc: `classify-queue submit` for a batch id the ledger never
//! received must refuse, name the id and append nothing. It used to record an
//! `ingest.batch_classified` event that covered nothing and exit 0, which left the real batches in
//! the queue and read like a success. A worker takes its ids from `classify-queue list`, so an id
//! the ledger holds no received batch for is a typo or another ledger's batch. The same rule as
//! acting on a decision that does not exist (hivemind-5t1uk). A batch that is already classified
//! was received, so sending it again is accepted as before.
//! This spawns the real binary, as an agent does.
//!
//! hivemind-d6tya: over `LOOMTRACER_API_URL` the same refusal arrives as the server's 404, and the
//! CLI used to exit 2 as "invalid command line input" for it. The help and both docs say exit 3,
//! and an agent that branches on the code must see one answer whether it asked a ledger or a
//! cell.
//!
//! hivemind-vf0iz: the server answers a token that maps to a tenant it does not hold with its own
//! 404 `unknown_tenant`, and the CLI reads it as the local ledger's storage refusal, exit 4. It
//! must not read as the refused batch id above: every call fails for that one reason.
//!
//! hivemind-vx9ma: over `LOOMTRACER_API_URL` the CLI sent no tenant and no actor, so a SQLite
//! server answered for tenant `local` as `service:api` whatever `--tenant`, `LOOMTRACER_TENANT` and
//! `--actor` said, while a local ledger honours all three. `list` showed another tenant's queue
//! and `submit` recorded under the wrong name. A token still decides both on the server.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use loomtracer::commands::{CommandContext, Commands};
use loomtracer::events::{Event, EventProvenance, EventType, TenantId};
use loomtracer::ledger::{EventLedger, SqliteEventLedger, SqliteUserStore};

fn unique_dir(label: &str) -> PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push(format!(
        "loomtracer-classify-queue-unknown-batch-{label}-{}",
        uuid::Uuid::new_v4()
    ));
    dir
}

/// `loomtracer --loomtracer-dir <dir> <args>` with no cell URL, no shared database, no
/// `LOOMTRACER_DIR` and the default daily cap, unless `envs` says otherwise.
fn loomtracer_with(dir: &Path, envs: &[(&str, &str)], args: &[&str]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_loomtracer"));
    command
        .env_remove("LOOMTRACER_API_URL")
        .env_remove("LOOMTRACER_DATABASE_URL")
        .env_remove("LOOMTRACER_DIR")
        .env_remove("LOOMTRACER_CLASSIFY_DAILY_CAP");
    for (key, value) in envs {
        command.env(key, value);
    }
    command
        .arg("--loomtracer-dir")
        .arg(dir)
        .args(args)
        .stdin(std::process::Stdio::null())
        .output()
        .expect("run loomtracer") // ubs:ignore: test-only; panicking is correct in tests
}

fn loomtracer(dir: &Path, args: &[&str]) -> Output {
    loomtracer_with(dir, &[], args)
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// A ledger under `dir` that has received `batch_ids`, the way `POST /v1/ingest` leaves one.
fn ledger_with_batches(dir: &Path, batch_ids: &[&str]) {
    ledger_with_batches_in(dir, &TenantId::local(), batch_ids);
}

/// [`ledger_with_batches`] in `tenant`, which the ledger must already hold.
fn ledger_with_batches_in(dir: &Path, tenant: &TenantId, batch_ids: &[&str]) {
    let ledger = SqliteEventLedger::open_for_tenant(dir, tenant).expect("open the ledger"); // ubs:ignore: test-only; panicking is correct in tests
    let commands = Commands::new_with_context(
        &ledger,
        CommandContext::new(tenant.clone(), EventProvenance::agent("agent:test")),
    );
    for batch_id in batch_ids {
        commands
            .record_ingest_batch("agent:test", batch_id, "claude", "session-x", Vec::new())
            .expect("receive the batch"); // ubs:ignore: test-only; panicking is correct in tests
    }
}

/// The events of `tenant` in the ledger under `dir`.
fn events_in(dir: &Path, tenant: &TenantId) -> Vec<Event> {
    let ledger = SqliteEventLedger::open(dir).expect("open the ledger"); // ubs:ignore: test-only; panicking is correct in tests
    ledger
        .read_for_tenant(tenant, 0, 10_000)
        .expect("read the ledger") // ubs:ignore: test-only; panicking is correct in tests
}

/// How many events tenant `local` of the ledger under `dir` holds.
fn event_count(dir: &Path) -> usize {
    events_in(dir, &TenantId::local()).len()
}

fn pending_total(dir: &Path) -> u64 {
    let listed = loomtracer(dir, &["classify-queue", "list", "--json", "--limit", "1"]);
    assert!(listed.status.success(), "{}", stderr(&listed));
    let queue: serde_json::Value =
        serde_json::from_str(&stdout(&listed)).expect("the queue is JSON"); // ubs:ignore: test-only; panicking is correct in tests
    queue["pending_total"].as_u64().expect("a pending_total") // ubs:ignore: test-only; panicking is correct in tests
}

fn submit(dir: &Path, batch_ids: &str) -> Output {
    submit_with(dir, &[], batch_ids)
}

fn submit_with(dir: &Path, envs: &[(&str, &str)], batch_ids: &str) -> Output {
    loomtracer_with(
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
    let capped = [("LOOMTRACER_CLASSIFY_DAILY_CAP", "1")];

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
// Over LOOMTRACER_API_URL (hivemind-d6tya)
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

/// A `loomtracer serve --port 0` process on a ledger directory, killed when dropped.
struct Served {
    child: Child,
    url: String,
}

impl Served {
    fn start(dir: &Path) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_loomtracer"))
            .env_remove("LOOMTRACER_API_KEY")
            .env_remove("LOOMTRACER_ADMIN_KEY")
            .env_remove("LOOMTRACER_DATABASE_URL")
            .env_remove("LOOMTRACER_BIND")
            .env_remove("LOOMTRACER_PORT")
            .env_remove("WORKOS_DOMAIN")
            .args(["-v", "--loomtracer-dir"])
            .arg(dir)
            .args(["serve", "--port", "0"])
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn loomtracer serve"); // ubs:ignore: test-only; panicking is correct in tests
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
/// holds no ledger, with only `LOOMTRACER_API_URL` pointing at the server.
fn submit_over(url: &str, own_dir: &Path, batch_ids: &str) -> Output {
    submit_with(own_dir, &[("LOOMTRACER_API_URL", url)], batch_ids)
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
// An unknown tenant over LOOMTRACER_API_URL (hivemind-vf0iz)
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
    loomtracer_with(
        own_dir,
        &[("LOOMTRACER_API_URL", url), ("LOOMTRACER_API_KEY", token)],
        &["classify-queue", "list"],
    )
}

fn submit_over_as(url: &str, token: &str, own_dir: &Path, batch_ids: &str) -> Output {
    loomtracer_with(
        own_dir,
        &[("LOOMTRACER_API_URL", url), ("LOOMTRACER_API_KEY", token)],
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

    let local_list = loomtracer(
        &local_dir,
        &["--tenant", GHOST_TENANT, "classify-queue", "list"],
    );
    let local_submit = loomtracer(
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

/// A server that is not Loomtracer's answers 404 in its own words, not as the ledger's refusal:
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

// ---------------------------------------------------------------------------
// --tenant and --actor over LOOMTRACER_API_URL (hivemind-vx9ma)
// ---------------------------------------------------------------------------

const ACME: &str = "acme";
const WORKER: &str = "agent:claude:worker";
const LOCAL_BATCH: &str = "local-session:0-1";
const ACME_BATCH: &str = "acme-session:0-1";

fn acme() -> TenantId {
    TenantId::new(ACME).expect("a tenant id") // ubs:ignore: test-only; panicking is correct in tests
}

/// A ledger under `dir` that holds one waiting batch in tenant `local` and one in `acme`.
fn ledger_with_a_batch_in_local_and_in_acme(dir: &Path) {
    ledger_with_batches(dir, &[LOCAL_BATCH]);
    SqliteEventLedger::open(dir)
        .expect("open the ledger") // ubs:ignore: test-only; panicking is correct in tests
        .create_tenant(&acme())
        .expect("register the tenant"); // ubs:ignore: test-only; panicking is correct in tests
    ledger_with_batches_in(dir, &acme(), &[ACME_BATCH]);
}

/// `loomtracer <args>` the way an agent on a cell runs it: from a folder of its own that holds no
/// ledger, with `LOOMTRACER_API_URL` (and whatever else `envs` adds) pointing at the server.
fn over_cell(url: &str, own_dir: &Path, envs: &[(&str, &str)], args: &[&str]) -> Output {
    let mut all_envs = vec![("LOOMTRACER_API_URL", url)];
    all_envs.extend_from_slice(envs);
    loomtracer_with(own_dir, &all_envs, args)
}

/// The batch ids a `classify-queue list --json` answer holds.
fn queued_ids(output: &Output) -> Vec<String> {
    assert!(output.status.success(), "{}", stderr(output));
    let queue: serde_json::Value =
        serde_json::from_str(&stdout(output)).expect("the queue is JSON"); // ubs:ignore: test-only; panicking is correct in tests
    queue["batches"]
        .as_array()
        .expect("a batches array") // ubs:ignore: test-only; panicking is correct in tests
        .iter()
        .map(|batch| {
            batch["batch_id"]
                .as_str()
                .expect("a batch_id") // ubs:ignore: test-only; panicking is correct in tests
                .to_owned()
        })
        .collect()
}

/// The `ingest.batch_classified` events of `tenant`, by the actor each names.
fn classified_by(dir: &Path, tenant: &TenantId) -> Vec<String> {
    events_in(dir, tenant)
        .into_iter()
        .filter(|event| event.event_type == EventType::IngestBatchClassified)
        .map(|event| event.actor_id)
        .collect()
}

#[test]
fn over_a_cell_url_list_shows_the_queue_of_the_tenant_the_flag_names() {
    let served_dir = unique_dir("served-list-tenant");
    ledger_with_a_batch_in_local_and_in_acme(&served_dir);
    let own_dir = unique_dir("own-list-tenant");
    let served = Served::start(&served_dir);

    let acmes = over_cell(
        &served.url,
        &own_dir,
        &[],
        &["--tenant", ACME, "classify-queue", "list", "--json"],
    );
    let locals = over_cell(
        &served.url,
        &own_dir,
        &[],
        &["classify-queue", "list", "--json"],
    );

    assert_eq!(queued_ids(&acmes), [ACME_BATCH], "--tenant acme");
    assert_eq!(queued_ids(&locals), [LOCAL_BATCH], "no --tenant is local");
    assert!(
        !own_dir.exists(),
        "the CLI made a ledger of its own beside the cell"
    );
}

#[test]
fn over_a_cell_url_list_takes_the_tenant_from_loomtracer_tenant_too() {
    let served_dir = unique_dir("served-list-env-tenant");
    ledger_with_a_batch_in_local_and_in_acme(&served_dir);
    let own_dir = unique_dir("own-list-env-tenant");
    let served = Served::start(&served_dir);

    let listed = over_cell(
        &served.url,
        &own_dir,
        &[("LOOMTRACER_TENANT", ACME)],
        &["classify-queue", "list", "--json"],
    );

    assert_eq!(queued_ids(&listed), [ACME_BATCH]);
}

#[test]
fn over_a_cell_url_submit_records_in_the_flagged_tenant_by_the_flagged_actor() {
    let served_dir = unique_dir("served-submit-tenant");
    ledger_with_a_batch_in_local_and_in_acme(&served_dir);
    let own_dir = unique_dir("own-submit-tenant");
    let served = Served::start(&served_dir);
    let submit_args = |global: &[&'static str]| -> Vec<&'static str> {
        let mut args = global.to_vec();
        args.extend([
            "classify-queue",
            "submit",
            "--batch-id",
            ACME_BATCH,
            "--captures",
            "[]",
        ]);
        args
    };

    // Tenant local never received acme's batch: that id is refused there, as on a local ledger.
    let wrong_tenant = over_cell(
        &served.url,
        &own_dir,
        &[],
        &submit_args(&["--actor", WORKER]),
    );
    assert_eq!(
        wrong_tenant.status.code(),
        Some(3),
        "{}",
        stderr(&wrong_tenant)
    );
    assert!(
        stderr(&wrong_tenant).contains(&format!("ingest batch does not exist: {ACME_BATCH}")),
        "{}",
        stderr(&wrong_tenant)
    );
    assert!(classified_by(&served_dir, &acme()).is_empty());

    let submitted = over_cell(
        &served.url,
        &own_dir,
        &[],
        &submit_args(&["--tenant", ACME, "--actor", WORKER]),
    );

    assert!(submitted.status.success(), "{}", stderr(&submitted));
    assert_eq!(
        classified_by(&served_dir, &acme()),
        [WORKER],
        "acme records the classification under --actor"
    );
    assert!(
        classified_by(&served_dir, &TenantId::local()).is_empty(),
        "tenant local recorded one"
    );
    let acme_queue = over_cell(
        &served.url,
        &own_dir,
        &[],
        &["--tenant", ACME, "classify-queue", "list", "--json"],
    );
    assert!(
        queued_ids(&acme_queue).is_empty(),
        "acme's batch left its queue"
    );
    let local_queue = over_cell(
        &served.url,
        &own_dir,
        &[],
        &["classify-queue", "list", "--json"],
    );
    assert_eq!(queued_ids(&local_queue), [LOCAL_BATCH], "local's did not");
}

/// A token names its tenant and its actor itself, so the flags cannot move it: the server keeps
/// ignoring the headers the CLI sends (an agent token must not be able to speak as someone else).
#[test]
fn over_a_cell_url_a_token_decides_the_tenant_and_the_actor_not_the_flags() {
    let served_dir = unique_dir("served-token-wins");
    ledger_with_a_batch_in_local_and_in_acme(&served_dir);
    let token = SqliteUserStore::open(&served_dir)
        .expect("open the user store") // ubs:ignore: test-only; panicking is correct in tests
        .mint_agent_token(ACME, "agent:test:acme", None)
        .expect("mint a token") // ubs:ignore: test-only; panicking is correct in tests
        .token_secret;
    let own_dir = unique_dir("own-token-wins");
    let served = Served::start(&served_dir);
    let envs = [("LOOMTRACER_API_KEY", token.as_str())];

    let listed = over_cell(
        &served.url,
        &own_dir,
        &envs,
        &["--tenant", "local", "classify-queue", "list", "--json"],
    );
    assert_eq!(
        queued_ids(&listed),
        [ACME_BATCH],
        "the token's tenant, not --tenant"
    );

    let submitted = over_cell(
        &served.url,
        &own_dir,
        &envs,
        &[
            "--tenant",
            "local",
            "--actor",
            WORKER,
            "classify-queue",
            "submit",
            "--batch-id",
            ACME_BATCH,
            "--captures",
            "[]",
        ],
    );
    assert!(submitted.status.success(), "{}", stderr(&submitted));
    assert_eq!(
        classified_by(&served_dir, &acme()),
        ["agent:test:acme"],
        "the token's actor, not --actor"
    );
    assert!(classified_by(&served_dir, &TenantId::local()).is_empty());
}
