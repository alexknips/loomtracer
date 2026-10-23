use std::fs;
use std::path::Path;
use std::sync::Barrier;
use std::thread;

use clap::Parser;
use loomtracer::cli::{run, Cli};
use loomtracer::ledger::SqliteEventLedger;
use loomtracer::projector::{memory::MemoryGraph, rebuild_graph_for_tenant};
use loomtracer::queries::{search_decisions_fts_with_context, QueryContext, SearchDecisionRequest};
use rusqlite::Connection;

#[allow(dead_code)]
#[path = "support/seed_data.rs"]
mod seed_data;

#[allow(dead_code)]
#[path = "support/multi_tenant.rs"]
mod multi_tenant;

use multi_tenant::{seed_tenant, TenantDataset};
use seed_data::{seed_to_dir, unique_temp_dir, TestResult};

/// Searches each tenant runs against the shared ledger file, back to back with the other
/// tenants' searches. The old shared index lost within a handful of rounds.
const SEARCH_ROUNDS: usize = 30;

#[test]
fn query_search_matches_seed_ledger_on_every_run() -> TestResult<()> {
    let first = unique_temp_dir("fts-first");
    let second = unique_temp_dir("fts-second");
    seed_to_dir(&first)?;
    seed_to_dir(&second)?;

    let first_output = run_seed_search(&first)?;
    let second_output = run_seed_search(&second)?;
    assert_eq!(decision_ids(&first_output), vec!["decision-020"]);
    assert_eq!(decision_ids(&first_output), decision_ids(&second_output));

    let _ = fs::remove_dir_all(&first);
    let _ = fs::remove_dir_all(&second);
    Ok(())
}

#[test]
fn query_search_writes_nothing_to_the_ledger_file() -> TestResult<()> {
    let dir = unique_temp_dir("fts-read-only");
    seed_to_dir(&dir)?;
    // Fold the seeding writes out of the WAL first, so the before and after states are the
    // same layout and any difference is the search's doing.
    checkpoint_ledger(&dir)?;
    let before = ledger_file_state(&dir)?;
    let tables_before = ledger_tables(&dir)?;

    let output = run_seed_search(&dir)?;
    assert_eq!(decision_ids(&output), vec!["decision-020"]);

    assert_eq!(
        ledger_file_state(&dir)?,
        before,
        "a search wrote to ledger.sqlite or its WAL"
    );
    assert_eq!(ledger_tables(&dir)?, tables_before);
    assert!(
        !ledger_tables(&dir)?.contains(&"decision_search_fts".to_owned()),
        "a search left its FTS index in the ledger file"
    );

    let _ = fs::remove_dir_all(&dir);
    Ok(())
}

#[test]
fn concurrent_tenant_searches_each_see_only_their_own_decisions() -> TestResult<()> {
    let dir = unique_temp_dir("fts-tenants");
    let ledger = SqliteEventLedger::open(&dir)?;
    let mut tenants = Vec::new();
    for name in ["alpha-corp", "beta-startup", "gamma-labs"] {
        ledger.create_tenant(&loomtracer::events::TenantId::new(name)?)?;
        tenants.push(seed_tenant(&ledger, name)?);
    }
    drop(ledger);

    // Every tenant's decisions live in the one ledger.sqlite; each tenant queries a word only
    // its own decisions carry, from its own connection, all starting together.
    let start = Barrier::new(tenants.len());
    let outcomes = thread::scope(|scope| {
        let workers: Vec<_> = tenants
            .iter()
            .map(|tenant| {
                let (dir, start) = (&dir, &start);
                scope.spawn(move || search_own_decisions(dir, tenant, start))
            })
            .collect();
        workers
            .into_iter()
            .map(|worker| {
                worker
                    .join()
                    .unwrap_or_else(|_| Err("search thread panicked".to_owned()))
            })
            .collect::<Vec<_>>()
    });
    for outcome in outcomes {
        outcome?;
    }

    let _ = fs::remove_dir_all(&dir);
    Ok(())
}

fn search_own_decisions(
    dir: &Path,
    tenant: &TenantDataset,
    start: &Barrier,
) -> std::result::Result<(), String> {
    let fail =
        |step: &str, error: &dyn std::fmt::Display| format!("{}: {step}: {error}", tenant.name);

    let ledger = SqliteEventLedger::open_for_tenant(dir, &tenant.tenant_id)
        .map_err(|error| fail("open ledger", &error))?;
    let graph = MemoryGraph::default();
    rebuild_graph_for_tenant(&ledger, &tenant.tenant_id, &graph)
        .map_err(|error| fail("rebuild graph", &error))?;
    let context = QueryContext::new(tenant.tenant_id.clone());
    let request = SearchDecisionRequest {
        query: Some(tenant.name.to_owned()),
        limit: 50,
        ..SearchDecisionRequest::default()
    };
    let mut expected = tenant.decision_ids.clone();
    expected.sort();

    start.wait();
    for round in 0..SEARCH_ROUNDS {
        let response = search_decisions_fts_with_context(&context, &ledger, &graph, &request)
            .map_err(|error| fail(&format!("round {round} search"), &error))?;
        let mut found: Vec<String> = response
            .data
            .items
            .iter()
            .map(|item| item.decision.id.clone())
            .collect();
        found.sort();
        if found != expected {
            return Err(format!(
                "{}: round {round} found {found:?}, expected exactly its own {expected:?}",
                tenant.name
            ));
        }
    }
    Ok(())
}

fn run_seed_search(seed_dir: &Path) -> TestResult<serde_json::Value> {
    let output = run(&Cli::parse_from([
        "loomtracer",
        "--loomtracer-dir",
        seed_dir.to_str().ok_or("seed path must be utf-8")?,
        "query",
        "search",
        "--q",
        "delta-mirror",
        "--limit",
        "5",
    ]))?;
    Ok(serde_json::from_str(&output)?)
}

fn decision_ids(output: &serde_json::Value) -> Vec<&str> {
    output["data"]["items"]
        .as_array()
        .expect("items array")
        .iter()
        .map(|item| item["decision"]["id"].as_str().expect("decision id string"))
        .collect()
}

fn checkpoint_ledger(dir: &Path) -> TestResult<()> {
    let connection = Connection::open(dir.join("ledger.sqlite"))?;
    connection.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))?;
    Ok(())
}

/// The ledger's main file bytes and its WAL length: a write shows up in one or the other.
fn ledger_file_state(dir: &Path) -> TestResult<(Vec<u8>, u64)> {
    let main = fs::read(dir.join("ledger.sqlite"))?;
    let wal = fs::metadata(dir.join("ledger.sqlite-wal")).map_or(0, |metadata| metadata.len());
    Ok((main, wal))
}

fn ledger_tables(dir: &Path) -> TestResult<Vec<String>> {
    let connection = Connection::open(dir.join("ledger.sqlite"))?;
    let mut statement =
        connection.prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")?;
    let names = statement
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(names)
}
