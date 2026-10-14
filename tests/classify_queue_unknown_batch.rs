//! Regression test for hivemind-odmnc: `classify-queue submit` for a batch id the ledger never
//! received must refuse, name the id and append nothing. It used to record an
//! `ingest.batch_classified` event that covered nothing and exit 0, which left the real batches in
//! the queue and read like a success. A worker takes its ids from `classify-queue list`, so an id
//! the ledger holds no received batch for is a typo or another ledger's batch. The same rule as
//! acting on a decision that does not exist (hivemind-5t1uk). A batch that is already classified
//! was received, so sending it again is accepted as before.
//! This spawns the real binary, as an agent does.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use hivemind::commands::{CommandContext, Commands};
use hivemind::events::{EventProvenance, TenantId};
use hivemind::ledger::{EventLedger, SqliteEventLedger};

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
