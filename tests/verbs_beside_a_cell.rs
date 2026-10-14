//! Regression test for hivemind-3wism, hivemind-nrk9f and hivemind-wp0n7: with `HIVEMIND_API_URL`
//! set, the verbs that open the ledger under `--hivemind-dir` must not use a local ledger
//! silently. Only `classify-queue` honours the variable. 3wism covered `emit`, `digest`, `review`,
//! `export` and `dump`; nrk9f covers `disagree`, `supersede`, `move`, `retitle`, `ground`, `ask`,
//! `import`, `restatements`, `map` and `quality-scan`; wp0n7 covers the `project` verbs,
//! `ingest slack-thread`, `tui` and `tenant create`. `emit` recorded the decision into a `./hivemind`
//! ledger it created and exited 0 with no word that the cell never got it, and the reads created
//! an empty ledger and answered from it. Same rule as `query` (hivemind-rmdi1,
//! tests/query_beside_a_cell.rs): with no local ledger the verb refuses and makes none; with one
//! it works and names the ledger it used.
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

// ---------------------------------------------------------------------------------------------
// hivemind-nrk9f: the ten verbs the section above left out
// ---------------------------------------------------------------------------------------------

/// What `disagree`, `retitle` and `ground` find by description, and `supersede` replaces.
const SEED_TITLE: &str = "Retry budget for the flaky upstream";

/// The verbs hivemind-3wism did not cover, named as the refusal names them. `restatements` is two
/// entries because `propose` reads and `apply` writes.
const OTHER_VERBS: [&str; 13] = [
    "disagree",
    "supersede",
    "move",
    "retitle",
    "ground",
    "reaffirm",
    "reexamine",
    "ask",
    "import",
    "restatements propose",
    "restatements apply",
    "map",
    "quality-scan",
];

/// Whether the verb writes to a ledger, so its refusal also says nothing was recorded.
fn writes(verb: &str) -> bool {
    !matches!(verb, "map" | "restatements propose" | "reexamine")
}

/// A ledger under `dir` that holds one decision about a retry budget, made the way a person makes
/// one: a local capture with no cell URL.
fn make_ledger_with_decision(dir: &Path) {
    let made = hivemind(
        dir,
        None,
        &[
            "--actor",
            "human:seed-author",
            "emit",
            "decision.capture",
            "--title",
            SEED_TITLE,
            "--rationale",
            "Three attempts is what the upstream tolerates today",
            "--topic-keys",
            "retries",
            "--options",
            "Keep 3,Raise to 5",
            "--chose",
            "Keep 3",
            "--rests-on-assumption",
            "The upstream stays flaky",
            "--binds-nothing",
            "--cheap-to-undo",
        ],
    );
    assert!(made.status.success(), "{}", stderr(&made));
    assert!(dir.join("ledger.sqlite").is_file());
}

/// A Markdown note with one `Decision:` block, for `import documents`. Lives beside, not inside,
/// the ledger folder, which a refused verb must leave without a trace.
fn write_decision_note(label: &str) -> PathBuf {
    let dir = unique_dir(label);
    std::fs::create_dir_all(&dir).expect("create the note folder"); // ubs:ignore: test-only; panicking is correct in tests
    let note = dir.join("notes.md");
    std::fs::write(
        &note,
        "Decision:\n  id: nrk9f-import\n  title: Import a note under a cell url\n  status: proposed\n  \
         actor: actor:alice\n  topic_keys: imports\n  rationale: The note is only here to be imported by the test.\n  \
         options:\n    - one\n    - two\n  chose: one\n",
    )
    .expect("write the decision note"); // ubs:ignore: test-only; panicking is correct in tests
    note
}

/// One invocation of `verb` that parses and, against `make_ledger_with_decision`, succeeds. The
/// acting actor is not the one that captured the decision: nobody can disagree with their own.
fn other_verb_args(verb: &str, note: &Path) -> Vec<String> {
    let note = note.display().to_string();
    let args: Vec<&str> = match verb {
        "disagree" => vec![
            "disagree",
            "retry budget",
            "--reason",
            "Three attempts is too few",
        ],
        "supersede" => vec![
            "supersede",
            "retry budget",
            "--title",
            "Raise the retry budget to five attempts",
            "--rationale",
            "The flaky upstream needs more attempts than three to succeed reliably",
            "--topic-keys",
            "retries",
            "--options",
            "Keep 3 attempts,Raise to 5 attempts",
            "--chose",
            "Raise to 5 attempts",
            "--rests-on-assumption",
            "The upstream stays flaky",
            "--binds-nothing",
            "--cheap-to-undo",
        ],
        // A successful `not_found`: a move needs a project that exists, and the point here is that
        // the verb ran and said whose ledger it ran against.
        "move" => vec!["move", "no-such-decision-needle", "--to", "pricing"],
        "retitle" => vec!["retitle", "retry budget", "--to", "Retry budget is five"],
        "ground" => vec![
            "ground",
            "retry budget",
            "--rests-on-assumption",
            "The upstream stays flaky",
        ],
        // A successful `not_found`, like `move`: the point is that the verb ran and said whose
        // ledger it ran against.
        "reaffirm" => vec![
            "reaffirm",
            "no-such-decision-needle",
            "--because",
            "Checked it again",
        ],
        "reexamine" => vec!["reexamine"],
        "ask" => vec!["ask", "How many retries does the upstream need?"],
        "import" => vec!["import", "documents", &note],
        "restatements propose" => vec!["restatements", "propose"],
        "restatements apply" => vec!["restatements", "apply", "--all"],
        "map" => vec!["map"],
        "quality-scan" => vec!["quality-scan", "--dry-run"],
        other => panic!("no invocation for {other}"), // ubs:ignore: test-only; panicking is correct in tests
    };
    ["--actor", "human:reviewer", "--json"]
        .into_iter()
        .chain(args)
        .map(str::to_owned)
        .collect()
}

/// The notice an answer carries: a `notice` key on a JSON object, else a `notice:` line on stdout
/// or stderr (a JSON array has no room for either, so its notice is on stderr).
fn notice_of(output: &Output) -> String {
    let answer = stdout(output);
    if let Some(notice) = serde_json::from_str::<serde_json::Value>(&answer)
        .ok()
        .and_then(|value| value["notice"].as_str().map(str::to_owned))
    {
        return notice;
    }
    let errors = stderr(output);
    let line = answer
        .lines()
        .chain(errors.lines())
        .find_map(|line| line.strip_prefix("notice: ").map(str::to_owned));
    line.unwrap_or_default()
}

#[test]
fn a_cell_url_with_no_local_ledger_refuses_the_ten_other_verbs_and_creates_nothing() {
    let note = write_decision_note("refuse-note");
    for verb in OTHER_VERBS {
        let dir = unique_dir(&format!("other-fresh-{}", verb.replace(' ', "-")));
        let args = other_verb_args(verb, &note);

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
            "{verb} says what it uses and that nothing was made: {message}"
        );
        assert_eq!(
            message.contains("nothing was recorded"),
            writes(verb),
            "{verb} says the write did not happen exactly when it writes: {message}"
        );
        assert!(
            message.contains(&dir.display().to_string()),
            "{verb} names the folder it did not use: {message}"
        );
        assert!(
            message.contains(&format!("{CELL}/")),
            "{verb} points at the cell's own route for the same thing: {message}"
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
    }
}

#[test]
fn a_refusal_names_the_cells_route_for_what_the_verb_does() {
    let note = write_decision_note("route-note");
    for (verb, route) in [
        ("disagree", "/v1/decisions/{id}/disagreements"),
        ("supersede", "/v1/decisions/{id}/supersessions"),
        ("move", "move_decision"),
        ("retitle", "retitle_decision"),
        ("ground", "ground_decision"),
        ("reaffirm", "/v1/decisions/{id}/reaffirmations"),
        ("reexamine", "get_reexamine_items"),
        ("ask", "request_decision"),
        ("restatements propose", "/v1/restatements/proposals"),
        ("restatements apply", "/v1/decisions/{id}/restatements"),
        ("map", "/v1/decisions/map"),
        ("quality-scan", "scan_decision_quality"),
    ] {
        let dir = unique_dir("route");
        let args = other_verb_args(verb, &note);

        let output = hivemind(&dir, Some(CELL), &as_strs(&args));

        assert!(
            stderr(&output).contains(route),
            "{verb} names {route}: {}",
            stderr(&output)
        );
    }
}

#[test]
fn the_ten_other_verbs_with_a_local_ledger_work_and_say_which_ledger_they_used() {
    let note = write_decision_note("works-note");
    for verb in OTHER_VERBS {
        let dir = unique_dir(&format!("other-existing-{}", verb.replace(' ', "-")));
        make_ledger_with_decision(&dir);
        let args = other_verb_args(verb, &note);

        let output = hivemind(&dir, Some(CELL), &as_strs(&args));

        assert!(
            output.status.success(),
            "{verb}: stdout: {} stderr: {}",
            stdout(&output),
            stderr(&output)
        );
        let notice = notice_of(&output);
        assert!(
            notice.contains(CELL) && notice.contains(&format!("`hivemind {verb}`")),
            "{verb} names the cell and itself: {notice:?} in {}",
            stdout(&output)
        );
        assert!(
            notice.contains(&dir.display().to_string()),
            "{verb} names the ledger it used: {notice}"
        );
        assert!(
            notice.contains("does not use that server")
                || notice.contains("does not read that server"),
            "{verb} says the cell was not involved: {notice}"
        );
    }
}

#[test]
fn a_write_with_a_local_ledger_really_lands_in_that_ledger() {
    // The notice says anything recorded went to the local ledger. Prove it did, for the verbs that
    // change a decision, by reading the title and status back without the cell URL.
    let dir = unique_dir("lands-locally");
    make_ledger_with_decision(&dir);
    let retitled = hivemind(
        &dir,
        Some(CELL),
        &[
            "--actor",
            "human:reviewer",
            "--json",
            "retitle",
            "retry budget",
            "--to",
            "Retry budget is five",
        ],
    );
    assert!(retitled.status.success(), "{}", stderr(&retitled));
    let disagreed = hivemind(
        &dir,
        Some(CELL),
        &[
            "--actor",
            "human:reviewer",
            "--json",
            "disagree",
            "retry budget",
            "--reason",
            "Three attempts is too few",
        ],
    );
    assert!(disagreed.status.success(), "{}", stderr(&disagreed));

    let read = hivemind(
        &dir,
        None,
        &["query", "search_decisions", "--q", "retry budget"],
    );

    let read = stdout(&read);
    assert!(
        read.contains("Retry budget is five") && read.contains("contested"),
        "the retitle and the disagreement are in the local ledger: {read}"
    );
}

#[test]
fn a_text_answer_ends_with_the_notice_line_and_is_otherwise_unchanged() {
    let dir = unique_dir("text-ask");
    make_ledger_with_decision(&dir);

    let with_cell = hivemind(
        &dir,
        Some(CELL),
        &["--actor", "human:reviewer", "ask", "How many retries?"],
    );
    let without_cell = hivemind(
        &dir,
        None,
        &["--actor", "human:reviewer", "ask", "How many retries?"],
    );

    assert!(with_cell.status.success(), "{}", stderr(&with_cell));
    let with_cell = stdout(&with_cell);
    let (answer, notice) = with_cell
        .trim_end()
        .rsplit_once('\n')
        .expect("an answer line and a notice line"); // ubs:ignore: test-only; panicking is correct in tests
    assert!(
        notice.starts_with("notice: HIVEMIND_API_URL is set") && notice.contains(CELL),
        "the last line is the notice: {with_cell}"
    );
    assert!(
        answer.starts_with("asked \"How many retries?\""),
        "the answer comes first and is the one the verb always gives: {answer}"
    );
    // The same call without the URL asks the same question again: still a request of its own.
    assert!(
        stdout(&without_cell).starts_with("asked \"How many retries?\"")
            && !stdout(&without_cell).contains("notice:"),
        "{}",
        stdout(&without_cell)
    );
}

#[test]
fn map_with_several_alphas_keeps_stdout_a_json_array_and_puts_the_notice_on_stderr() {
    let dir = unique_dir("map-alphas");
    make_ledger_with_decision(&dir);

    let output = hivemind(
        &dir,
        Some(CELL),
        &["map", "--alpha", "0.2", "--alpha", "0.8"],
    );

    assert!(output.status.success(), "{}", stderr(&output));
    let maps: serde_json::Value = serde_json::from_slice(&output.stdout)
        .expect("map with several --alpha prints a JSON array"); // ubs:ignore: test-only; panicking is correct in tests
    assert_eq!(maps.as_array().map(Vec::len), Some(2), "{maps}");
    assert!(
        stderr(&output).contains("notice: HIVEMIND_API_URL is set")
            && stderr(&output).contains("`hivemind map`"),
        "stderr says the maps are the local ledger's: {}",
        stderr(&output)
    );
}

#[test]
fn import_prepare_documents_opens_no_ledger_so_it_is_not_refused() {
    let note = write_decision_note("prepare-note");
    let dir = unique_dir("prepare-ledger");
    let out = unique_dir("prepare-out");

    let output = hivemind(
        &dir,
        Some(CELL),
        &[
            "import",
            "prepare-documents",
            "--format",
            "text",
            "--output-dir",
            &out.display().to_string(),
            &note.display().to_string(),
        ],
    );

    assert!(
        output.status.success(),
        "stdout: {} stderr: {}",
        stdout(&output),
        stderr(&output)
    );
    assert!(
        !stderr(&output).contains("HIVEMIND_API_URL") && !stdout(&output).contains("notice"),
        "a verb that never opens a ledger has nothing to say about one"
    );
    assert!(!dir.exists(), "prepare-documents made a ledger");
}

#[test]
fn without_a_cell_url_the_ten_other_verbs_are_unchanged() {
    let note = write_decision_note("unchanged-note");
    for verb in OTHER_VERBS {
        let dir = unique_dir(&format!("other-local-{}", verb.replace(' ', "-")));
        make_ledger_with_decision(&dir);
        let args = other_verb_args(verb, &note);

        let output = hivemind(&dir, None, &as_strs(&args));

        assert!(
            output.status.success(),
            "{verb}: stdout: {} stderr: {}",
            stdout(&output),
            stderr(&output)
        );
        assert!(
            !stdout(&output).contains("HIVEMIND_API_URL")
                && !stderr(&output).contains("HIVEMIND_API_URL"),
            "{verb} mentions a cell nobody set: {} {}",
            stdout(&output),
            stderr(&output)
        );
    }
}

#[test]
fn a_shared_database_url_leaves_the_ten_other_verbs_alone() {
    let note = write_decision_note("database-note");
    for verb in OTHER_VERBS {
        let dir = unique_dir("other-database-url");
        let args = other_verb_args(verb, &note);

        // The URL points nowhere, so the verb fails on the connection; the point is that it is
        // not the cell refusal and made no local ledger.
        let output = Command::new(env!("CARGO_BIN_EXE_hivemind"))
            .env("HIVEMIND_API_URL", CELL)
            .env(
                "HIVEMIND_DATABASE_URL",
                "postgres://nobody@127.0.0.1:9/none",
            )
            .arg("--hivemind-dir")
            .arg(&dir)
            .args(&args)
            .stdin(std::process::Stdio::null())
            .output()
            .expect("run hivemind"); // ubs:ignore: test-only; panicking is correct in tests

        assert!(
            !stderr(&output).contains("never contacts that server"),
            "{verb}: a configured database is not the cell refusal: {}",
            stderr(&output)
        );
        assert!(!dir.exists(), "{verb} made a local ledger");
    }
}

// ---------------------------------------------------------------------------------------------
// hivemind-wp0n7: project, ingest slack-thread, tui and tenant create
// ---------------------------------------------------------------------------------------------

/// The verbs hivemind-nrk9f left out, named as the refusal names them.
const FOURTH_ROUND_VERBS: [&str; 12] = [
    "project register",
    "project link",
    "project unlink",
    "project anchor",
    "project declare-topic",
    "project list",
    "project show",
    "project decisions",
    "project use",
    "ingest slack-thread",
    "tui",
    "tenant create",
];

/// The same verbs without `tui`, which needs a terminal and a build with the `tui` feature, so it
/// cannot run to the end here.
fn runnable_fourth_round_verbs() -> impl Iterator<Item = &'static str> {
    FOURTH_ROUND_VERBS.into_iter().filter(|verb| *verb != "tui")
}

/// Whether the refusal says nothing was recorded: the verbs that write to the ledger. `project use`
/// only reads the registry and keeps its setting in a file, and `tui` only browses.
fn fourth_round_records(verb: &str) -> bool {
    !matches!(
        verb,
        "project list" | "project show" | "project decisions" | "project use" | "tui"
    )
}

/// A ledger under `dir` that holds three registered projects and one link, made the way a person
/// makes one: local commands with no cell URL.
fn make_ledger_with_projects(dir: &Path) {
    for args in [
        vec!["project", "register", "pricing"],
        vec!["project", "register", "billing"],
        vec!["project", "register", "core"],
        vec![
            "project", "link", "--from", "pricing", "--to", "billing", "--kind", "part_of",
        ],
    ] {
        let made = hivemind(dir, None, &args);
        assert!(made.status.success(), "{args:?}: {}", stderr(&made));
    }
    assert!(dir.join("ledger.sqlite").is_file());
}

fn slack_thread_fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("slack")
        .join("thread_with_mention.json")
}

/// One invocation of `verb` that parses and, against `make_ledger_with_projects`, succeeds. `tui`
/// takes no `--json`: it needs a terminal.
fn fourth_round_args(verb: &str) -> Vec<String> {
    let thread = slack_thread_fixture().display().to_string();
    let args: Vec<&str> = match verb {
        "project register" => vec!["project", "register", "newproj"],
        "project link" => vec![
            "project",
            "link",
            "--from",
            "core",
            "--to",
            "billing",
            "--kind",
            "depends_on",
        ],
        "project unlink" => vec![
            "project", "unlink", "--from", "pricing", "--to", "billing", "--kind", "part_of",
        ],
        "project anchor" => vec![
            "project", "anchor", "--handle", "pricing", "--kind", "channel", "--value", "#pricing",
        ],
        "project declare-topic" => vec!["project", "declare-topic", "pricing", "seats"],
        "project list" => vec!["project", "list"],
        "project show" => vec!["project", "show", "pricing"],
        "project decisions" => vec!["project", "decisions", "pricing"],
        "project use" => vec!["project", "use", "pricing"],
        "ingest slack-thread" => vec!["ingest", "slack-thread", "--file", &thread],
        "tui" => vec!["tui"],
        "tenant create" => vec!["tenant", "create", "acme"],
        other => panic!("no invocation for {other}"), // ubs:ignore: test-only; panicking is correct in tests
    };
    let prefix: &[&str] = if verb == "tui" {
        &["--actor", "human:reviewer"]
    } else {
        &["--actor", "human:reviewer", "--json"]
    };
    prefix
        .iter()
        .copied()
        .chain(args)
        .map(str::to_owned)
        .collect()
}

#[test]
fn a_cell_url_with_no_local_ledger_refuses_project_ingest_tui_and_tenant_create_and_creates_nothing(
) {
    for verb in FOURTH_ROUND_VERBS {
        let dir = unique_dir(&format!("fourth-fresh-{}", verb.replace(' ', "-")));
        let args = fourth_round_args(verb);

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
            "{verb} says what it uses and that nothing was made: {message}"
        );
        assert_eq!(
            message.contains("nothing was recorded"),
            fourth_round_records(verb),
            "{verb} says the write did not happen exactly when it writes: {message}"
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
    }
}

#[test]
fn a_refusal_names_the_cells_route_where_it_has_one_and_says_so_where_it_has_none() {
    for (verb, route) in [
        ("project register", "no HTTP route or MCP tool"),
        ("project link", "no HTTP route or MCP tool"),
        ("project unlink", "no HTTP route or MCP tool"),
        ("project anchor", "no HTTP route or MCP tool"),
        ("project declare-topic", "no HTTP route or MCP tool"),
        ("project list", "GET http://127.0.0.1:9/v1/graph"),
        ("project show", "GET http://127.0.0.1:9/v1/graph"),
        ("project decisions", "GET http://127.0.0.1:9/v1/graph"),
        ("project use", "GET http://127.0.0.1:9/v1/graph"),
        (
            "ingest slack-thread",
            "POST http://127.0.0.1:9/v1/decisions",
        ),
        ("tui", "GET http://127.0.0.1:9/v1/decisions/search"),
        ("tenant create", "POST http://127.0.0.1:9/v1/tenants"),
    ] {
        let dir = unique_dir("fourth-route");
        let args = fourth_round_args(verb);

        let output = hivemind(&dir, Some(CELL), &as_strs(&args));

        assert!(
            stderr(&output).contains(route),
            "{verb} says {route}: {}",
            stderr(&output)
        );
    }
}

#[test]
fn the_fourth_round_verbs_with_a_local_ledger_work_and_say_which_ledger_they_used() {
    for verb in runnable_fourth_round_verbs() {
        let dir = unique_dir(&format!("fourth-existing-{}", verb.replace(' ', "-")));
        make_ledger_with_projects(&dir);
        let args = fourth_round_args(verb);

        let output = hivemind(&dir, Some(CELL), &as_strs(&args));

        assert!(
            output.status.success(),
            "{verb}: stdout: {} stderr: {}",
            stdout(&output),
            stderr(&output)
        );
        let notice = notice_of(&output);
        assert!(
            notice.contains(CELL) && notice.contains(&format!("`hivemind {verb}`")),
            "{verb} names the cell and itself: {notice:?} in {}",
            stdout(&output)
        );
        assert!(
            notice.contains(&dir.display().to_string()),
            "{verb} names the ledger it used: {notice}"
        );
        assert!(
            notice.contains("does not use that server")
                || notice.contains("does not read that server"),
            "{verb} says the cell was not involved: {notice}"
        );
    }
}

#[test]
fn tui_with_a_local_ledger_gets_past_the_guard() {
    // It cannot run to the end here (no terminal, and the `tui` feature is optional), so the point
    // is that a folder holding a ledger is not the cell refusal.
    let dir = unique_dir("fourth-tui-existing");
    make_ledger_with_projects(&dir);

    let output = hivemind(&dir, Some(CELL), &["tui"]);

    assert!(
        !stderr(&output).contains("never contacts that server"),
        "a folder with a ledger is not refused: {}",
        stderr(&output)
    );
}

#[test]
fn a_write_with_a_local_ledger_really_lands_in_that_ledger_for_project_and_tenant_create() {
    let dir = unique_dir("fourth-lands");
    make_ledger_with_projects(&dir);
    for args in [
        vec!["project", "register", "newproj"],
        vec!["project", "use", "pricing"],
        vec!["tenant", "create", "acme"],
    ] {
        let output = hivemind(
            &dir,
            Some(CELL),
            &[&["--actor", "human:reviewer", "--json"][..], &args].concat(),
        );
        assert!(output.status.success(), "{args:?}: {}", stderr(&output));
    }

    let project = hivemind(&dir, None, &["--json", "project", "show", "newproj"]);
    let setting = hivemind(
        &dir,
        None,
        &["--actor", "human:reviewer", "project", "show", "--current"],
    );
    let tenant = hivemind(
        &dir,
        None,
        &["--tenant", "acme", "query", "search_decisions", "--q", "x"],
    );

    assert!(
        stdout(&project).contains("newproj") && !stdout(&project).contains("not_found"),
        "the registered project is in the local ledger: {}",
        stdout(&project)
    );
    assert!(
        stdout(&setting).contains("handle=pricing"),
        "the current-project setting was stored: {}",
        stdout(&setting)
    );
    assert!(
        tenant.status.success(),
        "the tenant was created in the local ledger: {}",
        stderr(&tenant)
    );
}

#[test]
fn a_text_answer_ends_with_the_notice_line_for_project_list_and_is_otherwise_unchanged() {
    let dir = unique_dir("fourth-text-list");
    make_ledger_with_projects(&dir);

    let with_cell = hivemind(&dir, Some(CELL), &["project", "list"]);
    let without_cell = hivemind(&dir, None, &["project", "list"]);

    assert!(with_cell.status.success(), "{}", stderr(&with_cell));
    let with_cell = stdout(&with_cell);
    let (answer, notice) = with_cell
        .trim_end()
        .rsplit_once('\n')
        .expect("an answer and a notice line"); // ubs:ignore: test-only; panicking is correct in tests
    assert!(
        notice.starts_with("notice: HIVEMIND_API_URL is set")
            && notice.contains(CELL)
            && notice.contains("`hivemind project list`"),
        "the last line is the notice: {with_cell}"
    );
    assert_eq!(
        answer,
        stdout(&without_cell).trim_end(),
        "the answer is the one the verb always gives"
    );
    assert!(!stdout(&without_cell).contains("notice:"));
}

#[test]
fn the_verbs_that_open_no_ledger_are_not_refused() {
    // Each of these is left out of the guard on purpose: it keeps no ledger under --hivemind-dir
    // (only a token or the current-project file), makes its own, or honours HIVEMIND_API_URL.
    let manifest_url = "https://example.test/slack";
    for (label, args) in [
        (
            "project show --current",
            vec!["project", "show", "--current"],
        ),
        ("project use --clear", vec!["project", "use", "--clear"]),
        ("quickstart", vec!["quickstart"]),
        (
            "slack-app manifest",
            vec!["slack-app", "manifest", "--request-url", manifest_url],
        ),
        (
            "connector auth",
            vec!["connector", "auth", "no-such-connector"],
        ),
        ("classify-queue list", vec!["classify-queue", "list"]),
        (
            "classify-queue submit",
            vec![
                "classify-queue",
                "submit",
                "--batch-id",
                "session-x:0-1",
                "--captures",
                "[]",
            ],
        ),
    ] {
        let dir = unique_dir("fourth-left-out");

        let output = hivemind(&dir, Some(CELL), &args);

        assert!(
            !stderr(&output).contains("never contacts that server"),
            "{label} is not the cell refusal: {}",
            stderr(&output)
        );
        assert!(
            !dir.join("ledger.sqlite").exists(),
            "{label} made a ledger under {}",
            dir.display()
        );
    }
}

#[test]
fn without_a_cell_url_the_fourth_round_verbs_are_unchanged() {
    for verb in runnable_fourth_round_verbs() {
        let dir = unique_dir(&format!("fourth-local-{}", verb.replace(' ', "-")));
        make_ledger_with_projects(&dir);
        let args = fourth_round_args(verb);

        let output = hivemind(&dir, None, &as_strs(&args));

        assert!(
            output.status.success(),
            "{verb}: stdout: {} stderr: {}",
            stdout(&output),
            stderr(&output)
        );
        assert!(
            !stdout(&output).contains("HIVEMIND_API_URL")
                && !stderr(&output).contains("HIVEMIND_API_URL"),
            "{verb} mentions a cell nobody set: {} {}",
            stdout(&output),
            stderr(&output)
        );
    }
}

#[test]
fn a_shared_database_url_leaves_the_fourth_round_verbs_alone() {
    for verb in FOURTH_ROUND_VERBS {
        let dir = unique_dir("fourth-database-url");
        let args = fourth_round_args(verb);

        // The URL points nowhere, so the verb fails on the connection; the point is that it is
        // not the cell refusal and made no local ledger.
        let output = Command::new(env!("CARGO_BIN_EXE_hivemind"))
            .env("HIVEMIND_API_URL", CELL)
            .env(
                "HIVEMIND_DATABASE_URL",
                "postgres://nobody@127.0.0.1:9/none",
            )
            .arg("--hivemind-dir")
            .arg(&dir)
            .args(&args)
            .stdin(std::process::Stdio::null())
            .output()
            .expect("run hivemind"); // ubs:ignore: test-only; panicking is correct in tests

        assert!(
            !stderr(&output).contains("never contacts that server"),
            "{verb}: a configured database is not the cell refusal: {}",
            stderr(&output)
        );
        assert!(!dir.exists(), "{verb} made a local ledger");
    }
}
