//! Regression test for hivemind-rmdi1: with `HIVEMIND_API_URL` set, `hivemind query` must not
//! answer silently from a local ledger it made up. Only `classify-queue` honours the variable;
//! `query` reads the ledger under `--hivemind-dir`, so a person on a cell who ran the CLI from a
//! fresh folder was told "0 decisions" by an empty `./hivemind/ledger.sqlite` the read created.
//! This spawns the real binary, as the person does.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const CELL: &str = "http://127.0.0.1:9";

fn unique_dir(label: &str) -> PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push(format!(
        "hivemind-query-beside-cell-{label}-{}",
        uuid::Uuid::new_v4()
    ));
    dir
}

/// `hivemind --hivemind-dir <dir> query recent_decisions ...`, with `HIVEMIND_API_URL` set to
/// `api_url` or removed, and no shared database in the environment.
fn recent_decisions(dir: &Path, api_url: Option<&str>) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_hivemind"));
    command.env_remove("HIVEMIND_DATABASE_URL");
    match api_url {
        Some(url) => command.env("HIVEMIND_API_URL", url),
        None => command.env_remove("HIVEMIND_API_URL"),
    };
    command
        .arg("--hivemind-dir")
        .arg(dir)
        .args(["query", "recent_decisions", "--since", "1h", "--limit", "5"])
        .output()
        .expect("run hivemind") // ubs:ignore: test-only; panicking is correct in tests
}

#[test]
fn a_cell_url_with_no_local_ledger_refuses_and_creates_nothing() {
    let dir = unique_dir("fresh");

    let output = recent_decisions(&dir, Some(CELL));

    assert!(
        !output.status.success(),
        "query answered from a ledger it made up: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(CELL),
        "the refusal names the cell: {stderr}"
    );
    assert!(
        stderr.contains("never contacts that server") && stderr.contains("none was created"),
        "the refusal says what query reads and that nothing was made: {stderr}"
    );
    assert!(
        stderr.contains("/v1/decisions/search"),
        "the refusal points at the cell's read route: {stderr}"
    );
    assert!(
        !dir.exists(),
        "the refused read must not leave a ledger directory behind"
    );
}

#[test]
fn a_cell_url_with_a_local_ledger_answers_and_says_it_is_the_local_ledgers() {
    let dir = unique_dir("existing");
    let made = recent_decisions(&dir, None);
    assert!(
        made.status.success(),
        "{}",
        String::from_utf8_lossy(&made.stderr)
    );

    let output = recent_decisions(&dir, Some(CELL));

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let answer: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("query prints a JSON object"); // ubs:ignore: test-only; panicking is correct in tests
    let notice = answer["notice"].as_str().unwrap_or_default(); // ubs:ignore: test-only; a missing notice fails the assertion below
    assert!(
        notice.contains(CELL)
            && notice.contains("local ledger")
            && notice.contains("does not read that server"),
        "the answer names the cell and the ledger it came from: {answer}"
    );
    assert!(
        notice.contains(&dir.display().to_string()),
        "the answer names the ledger directory: {notice}"
    );
}

#[test]
fn without_a_cell_url_a_local_query_is_unchanged() {
    let dir = unique_dir("local");

    let output = recent_decisions(&dir, None);

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let answer: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("query prints a JSON object"); // ubs:ignore: test-only; panicking is correct in tests
    assert!(
        answer.get("notice").is_none(),
        "a local query carries no cell notice: {answer}"
    );
    assert!(dir.join("ledger.sqlite").is_file());
}

#[test]
fn a_blank_cell_url_is_no_cell_url() {
    let dir = unique_dir("blank");

    let output = recent_decisions(&dir, Some("  "));

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
