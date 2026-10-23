//! Regression test for hivemind-i6ply: a read verb aimed at a folder that holds no ledger must
//! refuse, not make an empty ledger on the spot and answer "No decision found" with exit 0. That
//! answer is indistinguishable from a real ledger that has no match, and a mistyped
//! `--loomtracer-dir` produced it. The same family as hivemind-rmdi1, 3wism, nrk9f and wp0n7
//! (tests/query_beside_a_cell.rs, tests/verbs_beside_a_cell.rs), which refused the case where
//! `LOOMTRACER_API_URL` is set; this is the case where it is not. Reads against a ledger that
//! exists, even an empty one, answer as before, and writes keep making the ledger on their first
//! run.
//! This spawns the real binary, as the person does.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn unique_dir(label: &str) -> PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push(format!(
        "loomtracer-reads-without-ledger-{label}-{}",
        uuid::Uuid::new_v4()
    ));
    dir
}

/// `loomtracer --loomtracer-dir <dir> <args>` with no cell URL, no shared database and no
/// `LOOMTRACER_DIR` in the environment.
fn loomtracer(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_loomtracer"))
        .env_remove("LOOMTRACER_API_URL")
        .env_remove("LOOMTRACER_DATABASE_URL")
        .env_remove("LOOMTRACER_DIR")
        .arg("--loomtracer-dir")
        .arg(dir)
        .args(args)
        .stdin(std::process::Stdio::null())
        .output()
        .expect("run loomtracer") // ubs:ignore: test-only; panicking is correct in tests
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// A ledger under `dir` that holds no decision, made the way a person starts one.
fn make_empty_ledger(dir: &Path) {
    let made = loomtracer(dir, &["emit", "evidence.recorded", "--content", "seed"]);
    assert!(made.status.success(), "{}", stderr(&made));
    assert!(dir.join("ledger.sqlite").is_file());
}

/// One read verb: how the refusal names it, and the invocation.
struct Read {
    name: &'static str,
    args: Vec<String>,
}

fn read(name: &'static str, args: &[&str]) -> Read {
    Read {
        name,
        args: args.iter().map(|arg| (*arg).to_owned()).collect(),
    }
}

/// Every read verb. The first five are the ones the bead's repro named; they are all `query`
/// subcommands, and the others are the rest of the verbs that only answer from the ledger. `out`
/// is where `export` writes.
fn reads(out: &Path) -> Vec<Read> {
    let out = out.display().to_string();
    vec![
        read("query", &["query", "why", "retry budget"]),
        read("query", &["query", "verify", "retry budget"]),
        read("query", &["query", "recall", "retry budget"]),
        read("query", &["query", "search", "--q", "retry"]),
        read(
            "query",
            &["query", "situational", "--paths", "src/retry.rs"],
        ),
        read("query", &["query", "recent_decisions", "--since", "7d"]),
        read("digest", &["digest", "--window", "1d", "--summary"]),
        read("export", &["export", "--format", "markdown", "--out", &out]),
        read("dump", &["dump"]),
        read("map", &["map"]),
        read("restatements propose", &["restatements", "propose"]),
        read("reexamine", &["reexamine"]),
        // The capture plugin's Stop hook runs this at the end of a session in every project
        // (hivemind-odmnc).
        read("classify-queue list", &["classify-queue", "list"]),
        read(
            "classify-queue list",
            &["classify-queue", "list", "--json", "--limit", "1"],
        ),
        read("project list", &["project", "list"]),
        read("project show", &["project", "show", "pricing"]),
        read("project decisions", &["project", "decisions", "pricing"]),
    ]
}

fn as_strs(args: &[String]) -> Vec<&str> {
    args.iter().map(String::as_str).collect()
}

fn assert_refused(read: &Read, dir: &Path, output: &Output) {
    let label = read.args.join(" ");
    assert_eq!(
        output.status.code(),
        Some(2),
        "`{label}` must refuse with exit 2; stdout: {} stderr: {}",
        stdout(output),
        stderr(output)
    );
    let message = stderr(output);
    assert!(
        message.contains(&format!("`loomtracer {}`", read.name)),
        "`{label}` names the verb: {message}"
    );
    assert!(
        message.contains(&dir.display().to_string()),
        "`{label}` names the folder it looked at: {message}"
    );
    assert!(
        message.contains("none was created"),
        "`{label}` says it made no ledger: {message}"
    );
    assert!(
        !message.contains("LOOMTRACER_API_URL"),
        "`{label}` mentions a cell nobody set: {message}"
    );
    assert!(
        stdout(output).is_empty(),
        "`{label}` printed an answer: {}",
        stdout(output)
    );
}

#[test]
fn a_read_aimed_at_a_folder_that_does_not_exist_refuses_and_creates_nothing() {
    let out = unique_dir("missing-out");
    for read in reads(&out) {
        let dir = unique_dir("missing");

        let output = loomtracer(&dir, &as_strs(&read.args));

        assert_refused(&read, &dir, &output);
        assert!(
            !dir.exists(),
            "`{}` left a folder behind at {}",
            read.args.join(" "),
            dir.display()
        );
        assert!(!out.exists(), "`{}` wrote an export", read.args.join(" "));
    }
}

#[test]
fn a_read_aimed_at_a_folder_with_no_ledger_in_it_refuses_and_adds_nothing_to_it() {
    let out = unique_dir("empty-folder-out");
    for read in reads(&out) {
        let dir = unique_dir("empty-folder");
        std::fs::create_dir_all(&dir).expect("create the empty folder"); // ubs:ignore: test-only; panicking is correct in tests

        let output = loomtracer(&dir, &as_strs(&read.args));

        assert_refused(&read, &dir, &output);
        let left: Vec<_> = std::fs::read_dir(&dir)
            .expect("read the folder") // ubs:ignore: test-only; panicking is correct in tests
            .collect();
        assert!(
            left.is_empty(),
            "`{}` put {} entries in {}",
            read.args.join(" "),
            left.len(),
            dir.display()
        );
        assert!(!out.exists(), "`{}` wrote an export", read.args.join(" "));
    }
}

#[test]
fn the_bead_repro_a_mistyped_folder_is_not_an_answer_of_no_decision_found() {
    let real = unique_dir("real");
    make_empty_ledger(&real);
    let typo = real.with_file_name(format!(
        "{}-tpyo",
        real.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("x")
    ));

    let output = loomtracer(&typo, &["query", "recall", "retry budget"]);

    assert_eq!(output.status.code(), Some(2), "{}", stderr(&output));
    assert!(stdout(&output).is_empty(), "{}", stdout(&output));
    assert!(
        stderr(&output).contains(&typo.display().to_string()),
        "{}",
        stderr(&output)
    );
    assert!(!typo.exists(), "the typo'd folder was made");
}

#[test]
fn a_read_of_an_existing_ledger_that_holds_no_decision_still_answers() {
    let dir = unique_dir("existing-empty");
    make_empty_ledger(&dir);
    let out = unique_dir("existing-empty-out");

    for read in reads(&out) {
        let output = loomtracer(&dir, &as_strs(&read.args));

        assert!(
            output.status.success(),
            "`{}` must answer from an empty ledger: stdout: {} stderr: {}",
            read.args.join(" "),
            stdout(&output),
            stderr(&output)
        );
        assert!(
            !stderr(&output).contains("none was created"),
            "`{}` was refused: {}",
            read.args.join(" "),
            stderr(&output)
        );
    }
}

#[test]
fn a_write_keeps_making_the_ledger_on_its_first_run() {
    let dir = unique_dir("first-write");
    assert!(!dir.exists());

    let output = loomtracer(&dir, &["emit", "evidence.recorded", "--content", "first"]);

    assert!(output.status.success(), "{}", stderr(&output));
    assert!(dir.join("ledger.sqlite").is_file());
}

#[test]
fn a_shared_database_names_no_folder_so_a_read_is_not_refused_for_one() {
    let dir = unique_dir("database-url");

    // The URL points nowhere, so the read fails on the connection; the point is that it is not
    // the missing-folder refusal and made no local ledger. `classify-queue list` reads the
    // database `submit` writes to, not a local ledger beside it (hivemind-odmnc).
    for args in [
        vec!["query", "recall", "retry budget"],
        vec!["classify-queue", "list"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_loomtracer"))
            .env_remove("LOOMTRACER_API_URL")
            .env(
                "LOOMTRACER_DATABASE_URL",
                "postgres://nobody@127.0.0.1:9/none",
            )
            .arg("--loomtracer-dir")
            .arg(&dir)
            .args(&args)
            .stdin(std::process::Stdio::null())
            .output()
            .expect("run loomtracer"); // ubs:ignore: test-only; panicking is correct in tests

        assert!(
            !stderr(&output).contains("none was created"),
            "`{}`: a configured database is not the missing-folder refusal: {}",
            args.join(" "),
            stderr(&output)
        );
        assert!(!dir.exists(), "`{}` made a local ledger", args.join(" "));
    }
}
