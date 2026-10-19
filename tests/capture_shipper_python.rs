//! The passive-capture shipper (`capture/core.py`, `hook_ship.py`, `sidecar.py`) posts Claude Code
//! transcripts to `POST /v1/ingest`. Its tests are Python `unittest` files, and nothing else in CI
//! ran them: a regression in cursor handling or in keeping a failed batch would have merged green
//! (hivemind-v9r2n). This runs the whole suite under `cargo test`, so the default test run covers
//! the shipper, and fails with the suite's own output when one of its tests fails.

use std::io::ErrorKind;
use std::path::Path;
use std::process::Command;

#[test]
fn the_capture_shipper_python_suite_passes() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let run = Command::new("python3")
        .args(["-m", "unittest", "discover", "-s", "capture/tests"])
        .current_dir(root)
        // The suite imports the shipper from the repo; keep it from leaving `__pycache__` there.
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .output();
    let output = match run {
        Ok(output) => output,
        Err(err) if err.kind() == ErrorKind::NotFound => {
            eprintln!("skipping: the capture shipper's tests need python3 on PATH");
            return;
        }
        Err(err) => panic!("could not run python3 -m unittest: {err}"),
    };
    assert!(
        output.status.success(),
        "capture/tests failed ({}):\n--- stdout ---\n{}\n--- stderr ---\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}
