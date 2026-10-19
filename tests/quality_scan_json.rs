//! Regression test for hivemind-r2076: `hivemind --json quality-scan` printed its JSON report when
//! there was a finding to file and a plain sentence when there was none, so a scan on a schedule
//! (`hivemind --json --actor agent:x quality-scan --dry-run | jq .filed`) worked until the first
//! time the ledger was clean, which is the normal state. With `--json` the clean answer is the same
//! report with zero counts and no issues; the text mode keeps its sentence.
//! This spawns the real binary, as a schedule does.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn unique_dir(label: &str) -> PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push(format!(
        "hivemind-quality-scan-json-{label}-{}",
        uuid::Uuid::new_v4()
    ));
    dir
}

/// `hivemind --hivemind-dir <dir> <args>` with no cell URL, no shared database and no Linear key.
fn hivemind(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_hivemind"))
        .env_remove("HIVEMIND_API_URL")
        .env_remove("HIVEMIND_DATABASE_URL")
        .env_remove("HIVEMIND_DIR")
        .env_remove("HIVEMIND_LINEAR_API_KEY")
        .env_remove("HIVEMIND_LINEAR_TEAM_ID")
        .arg("--hivemind-dir")
        .arg(dir)
        .args(args)
        .stdin(std::process::Stdio::null())
        .output()
        .expect("run hivemind") // ubs:ignore: test-only; panicking is correct in tests
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// A ledger under `dir` that holds no decision, so a scan finds nothing to file.
fn make_clean_ledger(dir: &Path) {
    let made = hivemind(dir, &["emit", "evidence.recorded", "--content", "seed"]);
    assert!(made.status.success(), "{}", stderr(&made));
    assert!(dir.join("ledger.sqlite").is_file());
}

fn report_of(output: &Output) -> serde_json::Value {
    assert_eq!(output.status.code(), Some(0), "{}", stderr(output));
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        // ubs:ignore: test-only; panicking is correct in tests
        panic!(
            "--json must print a JSON report: {error}; stdout: {}",
            stdout(output)
        )
    })
}

#[test]
fn json_on_a_clean_ledger_prints_the_report_with_nothing_filed() {
    let dir = unique_dir("dry-run");
    make_clean_ledger(&dir);

    let output = hivemind(&dir, &["--json", "quality-scan", "--dry-run"]);

    let report = report_of(&output);
    assert_eq!(report["dry_run"], true, "{report}");
    assert_eq!(report["scanned"], 0, "{report}");
    assert_eq!(report["filed"], 0, "{report}");
    assert_eq!(report["truncated"], false, "{report}");
    assert_eq!(report["issues"], serde_json::json!([]), "{report}");
    assert!(
        report.get("notice").is_none(),
        "a clean ledger with no cell URL has nothing to add: {report}"
    );
}

#[test]
fn json_on_a_clean_ledger_files_nothing_and_needs_no_linear_key() {
    let dir = unique_dir("live");
    make_clean_ledger(&dir);

    // No `--dry-run`: with nothing to file the connector is never reached, so the missing Linear
    // key and team that would refuse a real run are not asked for.
    let output = hivemind(&dir, &["--json", "quality-scan"]);

    let report = report_of(&output);
    assert_eq!(report["dry_run"], false, "{report}");
    assert_eq!(report["filed"], 0, "{report}");
    assert_eq!(report["issues"], serde_json::json!([]), "{report}");
}

#[test]
fn the_text_mode_says_in_a_sentence_that_there_is_nothing_to_file() {
    let dir = unique_dir("text");
    make_clean_ledger(&dir);

    let output = hivemind(&dir, &["quality-scan", "--dry-run"]);

    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert_eq!(
        stdout(&output).trim_end(),
        "quality-scan: no unacknowledged finding — nothing to file"
    );
}
