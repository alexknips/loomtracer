//! One Rust toolchain everywhere (hivemind-yqv18).
//!
//! `rust-toolchain.toml` names the toolchain a local `cargo` builds with, the polecats' and the
//! refinery's included. CI installs the same one through `.github/actions/setup-rust` and the
//! Docker image builds with the `FROM rust:` tag in `Dockerfile`. A floating channel (`stable`,
//! `nightly`) is how a host's build drifted from CI's, and a nightly kept every build's
//! directory under `target/debug/build` instead of reusing it until the disk filled.

use std::fs;
use std::path::Path;

type TestResult<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn read_repo_file(relative: &str) -> TestResult<String> {
    Ok(fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join(relative),
    )?)
}

/// The `channel` of `rust-toolchain.toml`, read the way `setup-rust` reads it.
fn pinned_channel() -> TestResult<String> {
    let file = read_repo_file("rust-toolchain.toml")?;
    let value = file
        .lines()
        .find_map(|line| line.trim().strip_prefix("channel"))
        .and_then(|rest| rest.trim().strip_prefix('='))
        .map(|rest| rest.trim().trim_matches('"').to_string())
        .ok_or("rust-toolchain.toml has no channel")?;
    Ok(value)
}

/// Every file under `dir`, as paths relative to the repository root.
fn files_under(dir: &str) -> TestResult<Vec<String>> {
    let mut found = Vec::new();
    for entry in fs::read_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join(dir))? {
        let entry = entry?;
        let relative = format!("{dir}/{}", entry.file_name().to_string_lossy());
        if entry.file_type()?.is_dir() {
            found.extend(files_under(&relative)?);
        } else {
            found.push(relative);
        }
    }
    Ok(found)
}

#[test]
fn the_pin_is_an_exact_version_with_the_components_ci_runs() -> TestResult<()> {
    let channel = pinned_channel()?;
    let parts: Vec<&str> = channel.split('.').collect();
    assert!(
        parts.len() == 3 && parts.iter().all(|part| part.parse::<u32>().is_ok()),
        "rust-toolchain.toml channel must be an exact X.Y.Z version, not a floating one; got {channel:?}"
    );
    let file = read_repo_file("rust-toolchain.toml")?;
    for component in ["rustfmt", "clippy"] {
        assert!(
            file.contains(&format!("\"{component}\"")),
            "rust-toolchain.toml must list the {component} component the CI gates run"
        );
    }
    Ok(())
}

#[test]
fn the_docker_image_builds_with_the_pinned_toolchain() -> TestResult<()> {
    let channel = pinned_channel()?;
    let dockerfile = read_repo_file("Dockerfile")?;
    let from_rust: Vec<&str> = dockerfile
        .lines()
        .filter(|line| line.starts_with("FROM rust:"))
        .collect();
    assert_eq!(
        from_rust.len(),
        1,
        "Dockerfile must have exactly one `FROM rust:` stage; got {from_rust:?}"
    );
    assert!(
        from_rust[0].starts_with(&format!("FROM rust:{channel}-")),
        "Dockerfile builds with {:?} but rust-toolchain.toml pins {channel}; bump both together",
        from_rust[0]
    );
    Ok(())
}

#[test]
fn ci_installs_rust_only_from_the_pinned_file() -> TestResult<()> {
    let setup = read_repo_file(".github/actions/setup-rust/action.yml")?;
    assert!(
        setup.contains("rust-toolchain.toml"),
        "the setup-rust action must read the toolchain from rust-toolchain.toml"
    );
    for path in files_under(".github")? {
        if path == ".github/actions/setup-rust/action.yml" || !path.ends_with(".yml") {
            continue;
        }
        let text = read_repo_file(&path)?;
        assert!(
            !text.contains("rust-toolchain@") && !text.contains("rustup toolchain install"),
            "{path} installs its own Rust toolchain; use ./.github/actions/setup-rust so CI follows rust-toolchain.toml"
        );
    }
    Ok(())
}
