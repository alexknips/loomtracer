//! Regression test for hivemind-f9l9y: `import documents` used to record every `Decision:` block
//! it found and only then check the extractor flags, so `--extractor-command` beside
//! `--llm-response` wrote the ledger and then failed with exit 2, the code that says "validation,
//! nothing recorded". `--extractor-arg` with no `--extractor-command` was refused beside
//! `--llm-response` but silently ignored on its own. The flag rules now sit on the arguments, so a
//! bad combination is refused while parsing, before the ledger is opened.
//! This spawns the real binary, as the person does.

use std::error::Error;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use hivemind::ledger::{EventLedger, SqliteEventLedger};

type TestResult = Result<(), Box<dyn Error>>;

fn unique_dir(label: &str) -> PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push(format!(
        "hivemind-import-extractor-flags-{label}-{}",
        uuid::Uuid::new_v4()
    ));
    dir
}

/// `hivemind --hivemind-dir <dir> <args>` with no cell URL, no shared database and no
/// `HIVEMIND_DIR` in the environment.
fn hivemind(dir: &Path, args: &[&str]) -> Result<Output, Box<dyn Error>> {
    Ok(Command::new(env!("CARGO_BIN_EXE_hivemind"))
        .env_remove("HIVEMIND_API_URL")
        .env_remove("HIVEMIND_DATABASE_URL")
        .env_remove("HIVEMIND_DIR")
        .arg("--hivemind-dir")
        .arg(dir)
        .args(args)
        .stdin(std::process::Stdio::null())
        .output()?)
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn event_count(dir: &Path) -> Result<usize, Box<dyn Error>> {
    Ok(SqliteEventLedger::open(dir)?.read(0, 10_000)?.len())
}

/// A folder holding the one note with a valid `Decision:` block that the import tests share.
fn notes_dir(root: &Path) -> Result<PathBuf, Box<dyn Error>> {
    let notes = root.join("notes");
    std::fs::create_dir_all(&notes)?;
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/documents/storage_decision.md");
    std::fs::copy(fixture, notes.join("storage_decision.md"))?;
    Ok(notes)
}

/// A ledger that already holds one event, made the way a person starts one.
fn ledger_with_one_event(dir: &Path) -> TestResult {
    let made = hivemind(dir, &["emit", "evidence.recorded", "--content", "seed"])?;
    if !made.status.success() {
        return Err(format!("could not start a ledger: {}", stderr(&made)).into());
    }
    Ok(())
}

/// The note's `Decision:` block really is importable, so a refusal below is the flags' doing.
#[test]
fn the_note_imports_when_the_flags_are_valid() -> TestResult {
    let root = unique_dir("control");
    let dir = root.join("hive");
    let notes = notes_dir(&root)?;
    ledger_with_one_event(&dir)?;
    let before = event_count(&dir)?;

    let imported = hivemind(
        &dir,
        &["import", "documents", notes.to_str().ok_or("utf-8 path")?],
    )?;
    if !imported.status.success() {
        return Err(format!("import failed: {}", stderr(&imported)).into());
    }
    let after = event_count(&dir)?;
    if after <= before {
        return Err(
            format!("the import wrote nothing: {before} events before, {after} after").into(),
        );
    }
    let _ = std::fs::remove_dir_all(&root);
    Ok(())
}

#[test]
fn conflicting_or_stray_extractor_flags_exit_2_and_record_nothing() -> TestResult {
    // The flag set, and what the refusal has to name so the person knows which flag to drop.
    let cases: [(&str, &[&str], &str); 3] = [
        (
            "command-and-response",
            &[
                "--extractor-command",
                "hivemind-document-extractor",
                "--llm-response",
                "response.json",
            ],
            "--llm-response",
        ),
        (
            "arg-and-response",
            &[
                "--extractor-arg=--verbose",
                "--llm-response",
                "response.json",
            ],
            "--llm-response",
        ),
        (
            "arg-alone",
            &["--extractor-arg=--verbose"],
            "--extractor-command",
        ),
    ];

    for (label, flags, names) in cases {
        let root = unique_dir(label);
        let dir = root.join("hive");
        let notes = notes_dir(&root)?;
        ledger_with_one_event(&dir)?;
        let before = event_count(&dir)?;

        let mut args = vec!["import", "documents", notes.to_str().ok_or("utf-8 path")?];
        args.extend_from_slice(flags);
        let refused = hivemind(&dir, &args)?;

        if refused.status.code() != Some(2) {
            return Err(format!(
                "{label}: expected exit 2, got {:?}\n{}",
                refused.status.code(),
                stderr(&refused)
            )
            .into());
        }
        if !stderr(&refused).contains(names) {
            return Err(format!(
                "{label}: the refusal should name {names}, got:\n{}",
                stderr(&refused)
            )
            .into());
        }
        let after = event_count(&dir)?;
        if after != before {
            return Err(format!(
                "{label}: the refused import changed the ledger: {before} events before, {after} after"
            )
            .into());
        }
        let _ = std::fs::remove_dir_all(&root);
    }
    Ok(())
}
