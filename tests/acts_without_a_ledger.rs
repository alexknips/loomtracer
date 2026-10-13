//! Regression test for hivemind-5t1uk: a verb that acts on a decision or project a ledger already
//! holds (`disagree`, `supersede`, `reaffirm`, `project link` and the rest) must refuse when it is
//! aimed at a folder that holds no ledger, not make an empty ledger there and answer not_found.
//! That left a stray ledger behind and sent the person looking for a missing decision instead of a
//! wrong folder. The sibling of hivemind-i6ply (tests/reads_without_a_ledger.rs), which refused the
//! read verbs. An existing ledger that holds nothing still gets the answer it always did, the verbs
//! that record a ledger's first event keep making it, and a configured `--database-url` names no
//! folder.
//! This spawns the real binary, as the person does.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn unique_dir(label: &str) -> PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push(format!(
        "hivemind-acts-without-ledger-{label}-{}",
        uuid::Uuid::new_v4()
    ));
    dir
}

/// `hivemind --hivemind-dir <dir> <args>` with no cell URL, no shared database and no
/// `HIVEMIND_DIR` in the environment.
fn hivemind(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_hivemind"))
        .env_remove("HIVEMIND_API_URL")
        .env_remove("HIVEMIND_DATABASE_URL")
        .env_remove("HIVEMIND_DIR")
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

/// A ledger under `dir` that holds no decision, made the way a person starts one.
fn make_empty_ledger(dir: &Path) {
    let made = hivemind(dir, &["emit", "evidence.recorded", "--content", "seed"]);
    assert!(made.status.success(), "{}", stderr(&made));
    assert!(dir.join("ledger.sqlite").is_file());
}

/// One verb that acts on what a ledger holds: how the refusal names it, the invocation, and what
/// an existing ledger that holds nothing answers it (exit code, and text in the answer).
struct Act {
    name: &'static str,
    args: Vec<&'static str>,
    code: i32,
    answer: &'static str,
}

fn act(name: &'static str, args: &[&'static str], code: i32, answer: &'static str) -> Act {
    Act {
        name,
        args: args.to_vec(),
        code,
        answer,
    }
}

/// Every verb the bead names. The decisions are named by description, so on a ledger with none
/// they get "no decision matches that description"; the project verbs name projects nobody
/// registered.
fn acts() -> Vec<Act> {
    const NO_DECISION: &str = "no decision matches that description";
    vec![
        act(
            "disagree",
            &["disagree", "retry budget", "--reason", "Too few"],
            0,
            NO_DECISION,
        ),
        act(
            "supersede",
            &[
                "supersede",
                "retry budget",
                "--title",
                "Retry five times",
                "--rationale",
                "Three retries lost two writes a week, so allow five.",
                "--bet",
                "--binds-nothing",
                "--undo-cost",
                "One config line",
            ],
            0,
            NO_DECISION,
        ),
        act(
            "move",
            &["move", "retry budget", "--to", "pricing"],
            0,
            NO_DECISION,
        ),
        act(
            "retitle",
            &["retitle", "retry budget", "--to", "Retry five times"],
            0,
            NO_DECISION,
        ),
        act(
            "ground",
            &["ground", "retry budget", "--bet"],
            0,
            NO_DECISION,
        ),
        act(
            "reaffirm",
            &["reaffirm", "retry budget", "--because", "Checked again"],
            0,
            NO_DECISION,
        ),
        act(
            "review",
            &["review", "--since", "1d"],
            0,
            "No matching decisions to review.",
        ),
        act(
            "quality-scan",
            &["quality-scan", "--dry-run"],
            0,
            "no unacknowledged finding",
        ),
        act(
            "restatements apply",
            &["restatements", "apply", "--all"],
            0,
            "0 link(s) recorded",
        ),
        act(
            "project link",
            &[
                "project", "link", "--from", "a-app", "--to", "b-app", "--kind", "part_of",
            ],
            3,
            "project does not exist: a-app",
        ),
        act(
            "project unlink",
            &[
                "project", "unlink", "--from", "a-app", "--to", "b-app", "--kind", "part_of",
            ],
            3,
            "no active part_of link from a-app to b-app",
        ),
        act(
            "project anchor",
            &[
                "project", "anchor", "--handle", "pricing", "--kind", "folder", "--value", "/x",
            ],
            3,
            "project does not exist: pricing",
        ),
        act(
            "project declare-topic",
            &["project", "declare-topic", "pricing", "seats"],
            3,
            "project not registered: pricing",
        ),
        act(
            "project use",
            &["project", "use", "pricing"],
            2,
            "no project called 'pricing'",
        ),
    ]
}

fn assert_refused(act: &Act, dir: &Path, output: &Output) {
    let label = act.args.join(" ");
    assert_eq!(
        output.status.code(),
        Some(2),
        "`{label}` must refuse with exit 2; stdout: {} stderr: {}",
        stdout(output),
        stderr(output)
    );
    let message = stderr(output);
    assert!(
        message.contains(&format!("`hivemind {}`", act.name)),
        "`{label}` names the verb: {message}"
    );
    assert!(
        message.contains(&dir.display().to_string()),
        "`{label}` names the folder it looked at: {message}"
    );
    assert!(
        message.contains("nothing was recorded and none was created"),
        "`{label}` says it recorded nothing and made no ledger: {message}"
    );
    assert!(
        !message.contains("HIVEMIND_API_URL"),
        "`{label}` mentions a cell nobody set: {message}"
    );
    assert!(
        stdout(output).is_empty(),
        "`{label}` printed an answer: {}",
        stdout(output)
    );
}

#[test]
fn an_act_aimed_at_a_folder_that_does_not_exist_refuses_and_creates_nothing() {
    for act in acts() {
        let dir = unique_dir("missing");

        let output = hivemind(&dir, &act.args);

        assert_refused(&act, &dir, &output);
        assert!(
            !dir.exists(),
            "`{}` left a folder behind at {}",
            act.args.join(" "),
            dir.display()
        );
    }
}

#[test]
fn an_act_aimed_at_a_folder_with_no_ledger_in_it_refuses_and_adds_nothing_to_it() {
    for act in acts() {
        let dir = unique_dir("empty-folder");
        std::fs::create_dir_all(&dir).expect("create the empty folder"); // ubs:ignore: test-only; panicking is correct in tests

        let output = hivemind(&dir, &act.args);

        assert_refused(&act, &dir, &output);
        let left: Vec<_> = std::fs::read_dir(&dir)
            .expect("read the folder") // ubs:ignore: test-only; panicking is correct in tests
            .collect();
        assert!(
            left.is_empty(),
            "`{}` put {} entries in {}",
            act.args.join(" "),
            left.len(),
            dir.display()
        );
    }
}

#[test]
fn the_bead_repro_a_mistyped_folder_is_not_a_missing_decision() {
    let real = unique_dir("real");
    make_empty_ledger(&real);
    let typo = real.with_file_name(format!(
        "{}-tpyo",
        real.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("x")
    ));

    let output = hivemind(&typo, &["disagree", "retry budget", "--reason", "Too few"]);

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
fn an_existing_ledger_that_holds_nothing_answers_each_act_as_it_did() {
    let dir = unique_dir("existing-empty");
    make_empty_ledger(&dir);

    for act in acts() {
        let output = hivemind(&dir, &act.args);

        let answer = format!("{}{}", stdout(&output), stderr(&output));
        assert_eq!(
            output.status.code(),
            Some(act.code),
            "`{}` must answer from an empty ledger as before: {answer}",
            act.args.join(" ")
        );
        assert!(
            answer.contains(act.answer),
            "`{}` must answer {:?}: {answer}",
            act.args.join(" "),
            act.answer
        );
        assert!(
            !answer.contains("none was created"),
            "`{}` was refused: {answer}",
            act.args.join(" ")
        );
    }
}

#[test]
fn a_verb_that_records_a_first_event_keeps_making_the_ledger_and_the_acts_then_run() {
    let dir = unique_dir("first-register");
    assert!(!dir.exists());

    for handle in ["a-app", "b-app"] {
        let output = hivemind(&dir, &["project", "register", handle]);
        assert!(output.status.success(), "{}", stderr(&output));
    }
    assert!(dir.join("ledger.sqlite").is_file());

    let linked = hivemind(
        &dir,
        &[
            "project", "link", "--from", "a-app", "--to", "b-app", "--kind", "part_of",
        ],
    );

    assert!(linked.status.success(), "{}", stderr(&linked));
}

#[test]
fn a_shared_database_names_no_folder_so_an_act_is_not_refused_for_one() {
    let dir = unique_dir("database-url");

    // The URL points nowhere, so the verb fails on the connection; the point is that it is not
    // the missing-folder refusal and made no local ledger.
    let output = Command::new(env!("CARGO_BIN_EXE_hivemind"))
        .env_remove("HIVEMIND_API_URL")
        .env(
            "HIVEMIND_DATABASE_URL",
            "postgres://nobody@127.0.0.1:9/none",
        )
        .arg("--hivemind-dir")
        .arg(&dir)
        .args(["disagree", "retry budget", "--reason", "Too few"])
        .stdin(std::process::Stdio::null())
        .output()
        .expect("run hivemind"); // ubs:ignore: test-only; panicking is correct in tests

    assert!(
        !stderr(&output).contains("none was created"),
        "a configured database is not the missing-folder refusal: {}",
        stderr(&output)
    );
    assert!(!dir.exists(), "a local ledger was made");
}
