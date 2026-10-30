//! A plugin's manifest version is what makes an installed copy update.
//!
//! `claude plugin update` and Codex both key the cache on `plugin.json`'s `version`: a plugin
//! whose files changed under the same version is "already at the latest version", and the user's
//! agent keeps the old skill text for good (hivemind-j56j7). So a change under `plugins/<name>/`
//! must come with a bump of that plugin's version in both manifests.
//!
//! `plugins/shipped-versions.json` records, per plugin, the version and a hash of every file
//! under its directory as shipped. The test fails when the files and the lock disagree, and when
//! the two manifests of a plugin disagree. It reads no git history, so it holds on a shallow CI
//! checkout too.
//!
//! After bumping `version` in `.claude-plugin/plugin.json` and `.codex-plugin/plugin.json`:
//!
//! ```text
//! LOOMTRACER_BLESS_PLUGIN_VERSIONS=1 cargo test --test plugin_versions
//! ```
//!
//! rewrites the lock. It refuses a plugin whose files changed under the version the lock already
//! records, so blessing cannot stand in for the bump.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

type TestResult<T> = std::result::Result<T, Box<dyn std::error::Error>>;

const LOCK: &str = "plugins/shipped-versions.json";
const MANIFESTS: [&str; 2] = [".claude-plugin/plugin.json", ".codex-plugin/plugin.json"];
const BLESS_ENV: &str = "LOOMTRACER_BLESS_PLUGIN_VERSIONS";

#[derive(Debug, PartialEq, Eq)]
struct Shipped {
    version: String,
    content_sha256: String,
}

#[test]
fn a_plugin_that_changed_ships_under_a_new_version() -> TestResult<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut current = BTreeMap::new();
    for (name, dir) in plugin_dirs(root)? {
        current.insert(
            name.clone(),
            Shipped {
                version: manifest_version(&name, &dir)?,
                content_sha256: content_hash(&dir)?,
            },
        );
    }
    let recorded = read_lock(&root.join(LOCK))?;

    if std::env::var_os(BLESS_ENV).is_some() {
        for (name, now) in &current {
            if let Some(then) = recorded.get(name) {
                if then.version == now.version && then.content_sha256 != now.content_sha256 {
                    return Err(unbumped(name, &now.version).into());
                }
            }
        }
        write_lock(&root.join(LOCK), &current)?;
        println!("blessed {} plugin versions in {LOCK}", current.len());
        return Ok(());
    }

    let mut failures = Vec::new();
    for (name, now) in &current {
        match recorded.get(name) {
            None => failures.push(format!(
                "plugins/{name} is not in {LOCK}. Record it with {BLESS_ENV}=1."
            )),
            Some(then) if then == now => {}
            Some(then) if then.version == now.version => {
                failures.push(unbumped(name, &now.version));
            }
            Some(then) => failures.push(format!(
                "plugins/{name} is at {} but {LOCK} records {}. Record it with {BLESS_ENV}=1 \
                 (cargo test --test plugin_versions).",
                now.version, then.version
            )),
        }
    }
    for name in recorded.keys().filter(|name| !current.contains_key(*name)) {
        failures.push(format!(
            "{LOCK} records plugins/{name}, which no longer exists. Drop it with {BLESS_ENV}=1."
        ));
    }
    if !failures.is_empty() {
        return Err(failures.join("\n").into());
    }
    Ok(())
}

fn unbumped(name: &str, version: &str) -> String {
    format!(
        "plugins/{name} changed, but its manifests still say {version}, the version {LOCK} records. \
         `claude plugin update` (and Codex) answer \"already at the latest version\" to an install \
         at {version} and keep the old files, so no installed copy gets this change. Bump \"version\" in \
         plugins/{name}/.claude-plugin/plugin.json and plugins/{name}/.codex-plugin/plugin.json \
         (where the plugin has one), then run {BLESS_ENV}=1 cargo test --test plugin_versions."
    )
}

fn plugin_dirs(root: &Path) -> TestResult<Vec<(String, PathBuf)>> {
    let mut dirs = Vec::new();
    for entry in fs::read_dir(root.join("plugins"))? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            dirs.push((
                entry.file_name().to_string_lossy().into_owned(),
                entry.path(),
            ));
        }
    }
    dirs.sort();
    Ok(dirs)
}

/// The version both manifests of a plugin carry; the plugin needs at least one manifest and they
/// must agree, since the Codex manifest moves with the Claude Code one.
fn manifest_version(name: &str, dir: &Path) -> TestResult<String> {
    let mut found: Vec<(&str, String)> = Vec::new();
    for manifest in MANIFESTS {
        let path = dir.join(manifest);
        if !path.exists() {
            continue;
        }
        let value: Value = serde_json::from_str(&fs::read_to_string(&path)?)?;
        let version = value
            .get("version")
            .and_then(Value::as_str)
            .filter(|version| !version.is_empty())
            .ok_or_else(|| format!("plugins/{name}/{manifest} has no \"version\""))?;
        found.push((manifest, version.to_owned()));
    }
    let Some((_, first)) = found.first() else {
        return Err(format!("plugins/{name} has no plugin.json in {MANIFESTS:?}").into());
    };
    if let Some((manifest, other)) = found.iter().find(|(_, version)| version != first) {
        return Err(format!(
            "plugins/{name}: {} says {first} but {manifest} says {other}; the two manifests move together",
            found[0].0
        )
        .into());
    }
    Ok(first.clone())
}

/// Hash of every file under `dir`, by relative path and bytes, in path order.
fn content_hash(dir: &Path) -> TestResult<String> {
    let mut files = Vec::new();
    collect_files(dir, dir, &mut files)?;
    files.sort();
    let mut hasher = Sha256::new();
    for relative in files {
        let bytes = fs::read(dir.join(&relative))?;
        hasher.update((relative.len() as u64).to_be_bytes());
        hasher.update(relative.as_bytes());
        hasher.update((bytes.len() as u64).to_be_bytes());
        hasher.update(&bytes);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn collect_files(base: &Path, dir: &Path, files: &mut Vec<String>) -> TestResult<()> {
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            collect_files(base, &path, files)?;
        } else {
            let relative = path.strip_prefix(base)?;
            files.push(
                relative
                    .components()
                    .map(|part| part.as_os_str().to_string_lossy())
                    .collect::<Vec<_>>()
                    .join("/"),
            );
        }
    }
    Ok(())
}

fn read_lock(path: &Path) -> TestResult<BTreeMap<String, Shipped>> {
    let Ok(text) = fs::read_to_string(path) else {
        return Ok(BTreeMap::new());
    };
    let value: Value = serde_json::from_str(&text)?;
    let mut recorded = BTreeMap::new();
    for (name, entry) in value.as_object().ok_or("lock is not a JSON object")? {
        let field = |key: &str| {
            entry
                .get(key)
                .and_then(Value::as_str)
                .map(str::to_owned)
                .ok_or_else(|| format!("{LOCK}: {name} has no \"{key}\""))
        };
        recorded.insert(
            name.clone(),
            Shipped {
                version: field("version")?,
                content_sha256: field("content_sha256")?,
            },
        );
    }
    Ok(recorded)
}

fn write_lock(path: &Path, current: &BTreeMap<String, Shipped>) -> TestResult<()> {
    let value: serde_json::Map<String, Value> = current
        .iter()
        .map(|(name, shipped)| {
            (
                name.clone(),
                json!({ "version": shipped.version, "content_sha256": shipped.content_sha256 }),
            )
        })
        .collect();
    fs::write(path, serde_json::to_string_pretty(&value)? + "\n")?;
    Ok(())
}
