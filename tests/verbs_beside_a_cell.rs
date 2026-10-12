//! Regression test for hivemind-3wism: with `HIVEMIND_API_URL` set, `emit`, `digest`, `review`,
//! `export` and `dump` must not use a local ledger silently. Only `classify-queue` honours the
//! variable. `emit` recorded the decision into a `./hivemind` ledger it created and exited 0 with
//! no word that the cell never got it, and the four reads created an empty ledger and answered
//! from it. Same rule as `query` (hivemind-rmdi1, tests/query_beside_a_cell.rs): with no local
//! ledger the verb refuses and makes none; with one it works and names the ledger it used.
//! This spawns the real binary, as the person does.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const CELL: &str = "http://127.0.0.1:9";

fn unique_dir(label: &str) -> PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push(format!(
        "hivemind-verbs-beside-cell-{label}-{}",
        uuid::Uuid::new_v4()
    ));
    dir
}

/// `hivemind --hivemind-dir <dir> <args>`, with `HIVEMIND_API_URL` set to `api_url` or removed,
/// and no shared database in the environment.
fn hivemind(dir: &Path, api_url: Option<&str>, args: &[&str]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_hivemind"));
    command.env_remove("HIVEMIND_DATABASE_URL");
    match api_url {
        Some(url) => command.env("HIVEMIND_API_URL", url),
        None => command.env_remove("HIVEMIND_API_URL"),
    };
    command
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

/// A ledger under `dir`, made the way a person makes one: a local capture with no cell URL.
fn make_local_ledger(dir: &Path) {
    let made = hivemind(
        dir,
        None,
        &["emit", "evidence.recorded", "--content", "seed"],
    );
    assert!(made.status.success(), "{}", stderr(&made));
    assert!(dir.join("ledger.sqlite").is_file());
}

/// The verbs the bead names, in the order the repro runs them.
const VERBS: [&str; 5] = ["emit", "digest", "review", "export", "dump"];

/// One invocation of `verb`, valid for a ledger that holds nothing. `out` is where `export`
/// writes.
fn verb_args(verb: &str, out: &Path) -> Vec<String> {
    let out = out.display().to_string();
    let args: &[&str] = match verb {
        "emit" => &["emit", "evidence.recorded", "--content", "to the cell"],
        "digest" => &["digest", "--window", "1d", "--summary"],
        "review" => &["review", "--since", "1d", "--json"],
        "export" => &["export", "--format", "markdown", "--out", &out],
        "dump" => &["dump"],
        other => panic!("no invocation for {other}"), // ubs:ignore: test-only; panicking is correct in tests
    };
    args.iter().map(|arg| (*arg).to_owned()).collect()
}

fn as_strs(args: &[String]) -> Vec<&str> {
    args.iter().map(String::as_str).collect()
}

#[test]
fn a_cell_url_with_no_local_ledger_refuses_every_verb_and_creates_nothing() {
    for verb in VERBS {
        let dir = unique_dir(&format!("fresh-{verb}"));
        let out = unique_dir(&format!("fresh-{verb}-out"));
        let args = verb_args(verb, &out);

        let output = hivemind(&dir, Some(CELL), &as_strs(&args));

        assert_eq!(
            output.status.code(),
            Some(2),
            "{verb} must refuse with exit 2; stdout: {} stderr: {}",
            stdout(&output),
            stderr(&output)
        );
        let message = stderr(&output);
        assert!(message.contains(CELL), "{verb} names the cell: {message}");
        assert!(
            message.contains(&format!("`hivemind {verb}`"))
                && message.contains("never contacts that server")
                && message.contains("none was created"),
            "{verb} says what it reads or writes and that nothing was made: {message}"
        );
        assert!(
            message.contains(&dir.display().to_string()),
            "{verb} names the folder it did not use: {message}"
        );
        assert!(
            stdout(&output).is_empty(),
            "{verb} printed an answer: {}",
            stdout(&output)
        );
        assert!(
            !dir.exists(),
            "{verb} left a ledger directory behind under {}",
            dir.display()
        );
        assert!(!out.exists(), "{verb} left an export directory behind");
    }
}

#[test]
fn a_refused_emit_says_nothing_was_recorded_and_where_the_cell_takes_it() {
    let dir = unique_dir("emit-refusal");

    let output = hivemind(
        &dir,
        Some(CELL),
        &["emit", "evidence.recorded", "--content", "x"],
    );

    let message = stderr(&output);
    assert!(
        message.contains("nothing was recorded"),
        "emit says the write did not happen: {message}"
    );
    assert!(
        message.contains(&format!("POST {CELL}/v1/decisions")),
        "emit points at the cell's write route: {message}"
    );
}

#[test]
fn the_bead_repro_decision_capture_is_refused_not_recorded() {
    let dir = unique_dir("capture-repro");

    let output = hivemind(
        &dir,
        Some(CELL),
        &[
            "emit",
            "decision.capture",
            "--title",
            "Checker 6R.7 capture with the API URL set",
            "--rationale",
            "Where does this land? Checker test",
            "--topic-keys",
            "checker",
            "--options",
            "Here,There",
            "--chose",
            "Here",
            "--rests-on-assumption",
            "checker",
            "--binds-nothing",
            "--cheap-to-undo",
        ],
    );

    assert_eq!(output.status.code(), Some(2), "{}", stderr(&output));
    assert!(stderr(&output).contains("nothing was recorded"));
    assert!(
        !dir.exists(),
        "the capture made a ledger the cell never sees"
    );
}

#[test]
fn emit_with_a_local_ledger_keeps_its_bare_id_on_stdout_and_names_the_ledger_on_stderr() {
    let dir = unique_dir("emit-existing");
    make_local_ledger(&dir);

    let output = hivemind(
        &dir,
        Some(CELL),
        &["emit", "evidence.recorded", "--content", "recorded locally"],
    );

    assert!(output.status.success(), "{}", stderr(&output));
    let id = stdout(&output);
    let id = id.trim();
    assert!(
        !id.is_empty() && !id.contains(char::is_whitespace) && !id.contains("notice"),
        "stdout is the bare id a script takes with $(hivemind emit ...): {id:?}"
    );
    let notice = stderr(&output);
    assert!(
        notice.contains(CELL)
            && notice.contains("recorded in the local ledger")
            && notice.contains("does not write to that server")
            && notice.contains(&dir.display().to_string()),
        "stderr names the cell and the ledger the evidence went to: {notice}"
    );
}

#[test]
fn emit_json_with_a_local_ledger_carries_the_notice_as_a_key() {
    let dir = unique_dir("emit-json");
    make_local_ledger(&dir);

    let output = hivemind(
        &dir,
        Some(CELL),
        &["--json", "emit", "evidence.recorded", "--content", "json"],
    );

    assert!(output.status.success(), "{}", stderr(&output));
    let answer: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("emit --json prints a JSON object"); // ubs:ignore: test-only; panicking is correct in tests
    let notice = answer["notice"].as_str().unwrap_or_default(); // ubs:ignore: test-only; a missing notice fails the assertion below
    assert!(
        notice.contains(CELL) && notice.contains("recorded in the local ledger"),
        "the answer says where the evidence went: {answer}"
    );
}

#[test]
fn dump_with_a_local_ledger_keeps_stdout_valid_dot_and_names_the_ledger_on_stderr() {
    let dir = unique_dir("dump-existing");
    make_local_ledger(&dir);

    let output = hivemind(&dir, Some(CELL), &["dump"]);

    assert!(output.status.success(), "{}", stderr(&output));
    let dot = stdout(&output);
    assert!(
        dot.trim_start().starts_with("digraph") && dot.trim_end().ends_with('}'),
        "stdout is still a DOT graph a pipe into `dot` can read: {dot}"
    );
    let notice = stderr(&output);
    assert!(
        notice.contains(CELL)
            && notice.contains("local ledger")
            && notice.contains("does not read that server")
            && notice.contains("/v1/graph"),
        "stderr names the cell and the ledger the graph came from: {notice}"
    );
}

#[test]
fn the_read_verbs_with_a_local_ledger_answer_and_say_it_is_the_local_ledgers() {
    let dir = unique_dir("reads-existing");
    make_local_ledger(&dir);
    let out = unique_dir("reads-existing-out");

    for verb in ["digest", "review", "export"] {
        let args = verb_args(verb, &out);

        let output = hivemind(&dir, Some(CELL), &as_strs(&args));

        assert!(output.status.success(), "{verb}: {}", stderr(&output));
        let answer = stdout(&output);
        let notice_line = answer
            .lines()
            .find(|line| line.contains("HIVEMIND_API_URL is set") && line.contains(CELL));
        let notice = notice_line.map(str::to_owned).unwrap_or_else(|| {
            // The JSON verbs carry it as a `notice` key rather than a line of its own.
            serde_json::from_str::<serde_json::Value>(&answer)
                .ok()
                .and_then(|value| value["notice"].as_str().map(str::to_owned))
                .unwrap_or_default()
        });
        assert!(
            notice.contains(CELL)
                && notice.contains(&format!("`hivemind {verb}`"))
                && (notice.contains("does not use that server")
                    || notice.contains("does not read that server")),
            "{verb} names the cell and says the answer is the local ledger's: {answer}"
        );
        assert!(
            notice.contains(&dir.display().to_string()),
            "{verb} names the ledger directory: {notice}"
        );
    }
    assert!(
        out.join("INDEX.md").is_file(),
        "export still wrote its files from the local ledger"
    );
}

#[test]
fn without_a_cell_url_the_verbs_are_unchanged() {
    let dir = unique_dir("local");
    let out = unique_dir("local-out");

    for verb in VERBS {
        let args = verb_args(verb, &out);
        let output = hivemind(&dir, None, &as_strs(&args));

        assert!(output.status.success(), "{verb}: {}", stderr(&output));
        assert!(
            !stdout(&output).contains("HIVEMIND_API_URL")
                && !stderr(&output).contains("HIVEMIND_API_URL"),
            "{verb} mentions a cell nobody set: {} {}",
            stdout(&output),
            stderr(&output)
        );
    }
    assert!(dir.join("ledger.sqlite").is_file());
}

#[test]
fn a_blank_cell_url_is_no_cell_url() {
    let dir = unique_dir("blank");

    let output = hivemind(
        &dir,
        Some("  "),
        &["emit", "evidence.recorded", "--content", "blank"],
    );

    assert!(output.status.success(), "{}", stderr(&output));
    assert!(dir.join("ledger.sqlite").is_file());
    assert!(!stderr(&output).contains("HIVEMIND_API_URL"));
}

#[test]
fn a_shared_database_url_is_left_alone() {
    let dir = unique_dir("database-url");

    // A configured shared database is a direct read of the cell's own data, so the verb goes to
    // it, not to the "no local ledger" refusal. The URL points nowhere, so the verb fails on the
    // connection, and the point is that it is not the cell refusal and made no local ledger.
    let mut command = Command::new(env!("CARGO_BIN_EXE_hivemind"));
    let output = command
        .env("HIVEMIND_API_URL", CELL)
        .env(
            "HIVEMIND_DATABASE_URL",
            "postgres://nobody@127.0.0.1:9/none",
        )
        .arg("--hivemind-dir")
        .arg(&dir)
        .args(["dump"])
        .stdin(std::process::Stdio::null())
        .output()
        .expect("run hivemind"); // ubs:ignore: test-only; panicking is correct in tests

    assert!(
        !stderr(&output).contains("never contacts that server"),
        "a configured database is not the cell refusal: {}",
        stderr(&output)
    );
}
