//! The capture plugin's `Stop` hook (`scripts/check-classify-queue.sh`) tells the person, at the
//! end of a session, that unclassified batches are waiting. It runs in every project the plugin is
//! installed in, so in a project nothing was recorded in it must do nothing: print nothing, exit
//! 0 and leave no `./loomtracer/` behind (hivemind-odmnc). `classify-queue list` used to make an
//! empty ledger in such a folder; the hook now looks for a ledger first, so it stays quiet
//! whichever loomtracer is installed, and the build under test refuses the folder too.
//! This runs the real script against the real binary, as Claude Code does.

#![cfg(unix)]

use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use loomtracer::commands::{CommandContext, Commands};
use loomtracer::events::{EventProvenance, TenantId};
use loomtracer::ledger::SqliteEventLedger;
use tempfile::TempDir;

type TestResult<T> = std::result::Result<T, Box<dyn std::error::Error>>;

const HOOK: &str = "plugins/loomtracer-capture/scripts/check-classify-queue.sh";

fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

/// A directory holding only a `loomtracer` that is the binary under test, to put on `PATH`.
fn bin_dir_with_loomtracer() -> TestResult<TempDir> {
    let bin = TempDir::new()?;
    symlink(
        env!("CARGO_BIN_EXE_loomtracer"),
        bin.path().join("loomtracer"),
    )?;
    Ok(bin)
}

/// Runs the hook from `project` with the binary under test first on `PATH`, no cell, no shared
/// database and no `LOOMTRACER_DIR`, then `envs`.
fn run_hook(project: &Path, bin: &Path, envs: &[(&str, &Path)]) -> TestResult<Output> {
    let path = format!("{}:{}", bin.display(), std::env::var("PATH")?);
    let mut command = Command::new(root().join(HOOK));
    command
        .current_dir(project)
        .env("PATH", path)
        .env_remove("LOOMTRACER_API_URL")
        .env_remove("LOOMTRACER_DATABASE_URL")
        .env_remove("LOOMTRACER_DIR")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (key, value) in envs {
        command.env(key, value);
    }
    Ok(command.output()?)
}

fn assert_silent_success(output: &Output) {
    assert!(output.status.success(), "{output:?}");
    assert!(
        output.stdout.is_empty(),
        "the hook printed: {}",
        String::from_utf8_lossy(&output.stdout)
    );
}

/// A ledger under `dir` that received `batch_ids`, the way `POST /v1/ingest` leaves one.
fn ledger_with_batches(dir: &Path, batch_ids: &[&str]) -> TestResult<()> {
    let ledger = SqliteEventLedger::open(dir)?;
    let commands = Commands::new_with_context(
        &ledger,
        CommandContext::new(TenantId::local(), EventProvenance::agent("agent:test")),
    );
    for batch_id in batch_ids {
        commands.record_ingest_batch("agent:test", batch_id, "claude", "session-x", Vec::new())?;
    }
    Ok(())
}

fn jq_is_installed() -> bool {
    ["/usr/bin/jq", "/bin/jq"]
        .iter()
        .any(|path| Path::new(path).is_file())
}

#[test]
fn in_a_project_with_no_ledger_the_hook_prints_nothing_and_creates_nothing() -> TestResult<()> {
    let bin = bin_dir_with_loomtracer()?;
    let project = TempDir::new()?;

    let output = run_hook(project.path(), bin.path(), &[])?;

    assert_silent_success(&output);
    let left: Vec<PathBuf> = fs::read_dir(project.path())?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<std::io::Result<_>>()?;
    assert!(left.is_empty(), "the hook left {left:?} in the project");
    Ok(())
}

#[test]
fn aimed_at_a_missing_folder_by_loomtracer_dir_the_hook_makes_no_folder() -> TestResult<()> {
    let bin = bin_dir_with_loomtracer()?;
    let project = TempDir::new()?;
    let typo = project.path().join("loomtracer-tpyo");

    let output = run_hook(
        project.path(),
        bin.path(),
        &[("LOOMTRACER_DIR", typo.as_path())],
    )?;

    assert_silent_success(&output);
    assert!(!typo.exists(), "the hook made {typo:?}");
    Ok(())
}

#[test]
fn the_build_under_test_refuses_the_folder_whatever_the_hook_does() -> TestResult<()> {
    // The hook's own look-before-asking is for older builds; this one refuses a folder with no
    // ledger itself, so the listing the hook runs leaves nothing behind either.
    let project = TempDir::new()?;
    let folder = project.path().join("loomtracer");

    let listed = Command::new(env!("CARGO_BIN_EXE_loomtracer"))
        .env_remove("LOOMTRACER_API_URL")
        .env_remove("LOOMTRACER_DATABASE_URL")
        .env_remove("LOOMTRACER_DIR")
        .arg("--loomtracer-dir")
        .arg(&folder)
        .args(["classify-queue", "list", "--json", "--limit", "1"])
        .stdin(Stdio::null())
        .output()?;

    assert_eq!(listed.status.code(), Some(2), "{listed:?}");
    assert!(listed.stdout.is_empty());
    assert!(!folder.exists(), "the listing made {folder:?}");
    Ok(())
}

#[test]
fn with_a_ledger_and_nothing_queued_the_hook_is_silent() -> TestResult<()> {
    let bin = bin_dir_with_loomtracer()?;
    let project = TempDir::new()?;
    ledger_with_batches(&project.path().join("loomtracer"), &[])?;

    let output = run_hook(project.path(), bin.path(), &[])?;

    assert_silent_success(&output);
    Ok(())
}

#[test]
fn with_batches_waiting_the_hook_still_says_so() -> TestResult<()> {
    if !jq_is_installed() {
        eprintln!("skipping: the hook needs jq in /usr/bin or /bin");
        return Ok(());
    }
    let bin = bin_dir_with_loomtracer()?;
    let project = TempDir::new()?;
    ledger_with_batches(
        &project.path().join("loomtracer"),
        &["session-x:0-1", "session-x:1-2"],
    )?;

    let output = run_hook(project.path(), bin.path(), &[])?;

    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim_end(),
        "[loomtracer] classify-queue: 2 batch(es) pending — run /classify-queue to drain"
    );
    Ok(())
}
