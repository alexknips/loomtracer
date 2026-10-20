// Parent module gates this file with #[cfg(test)]; repeat the marker so UBS can filter test-only assertions.
#[cfg(test)]
use std::fs;
use std::path::PathBuf;
use std::sync::{mpsc, Arc, Barrier};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use rusqlite::OptionalExtension;
use uuid::Uuid;

use crate::commands::{
    CommandContext, Commands, DecisionProposalEventUuids, DecisionProposalInput, FloorInput,
    Grounding,
};
use crate::error::CommandError;
use crate::events::{Event, EventProvenance, TenantId};
use crate::ledger::contract_tests::{
    assert_dedup_by_event_uuid, assert_event_ids_for_uuids, assert_grounding_events_round_trip,
    assert_monotonic_append, assert_read_offset_and_limit, assert_replay_from_zero_in_order,
    assert_replay_into_non_empty_ledger, assert_typed_and_id_reads, make_event,
};
use crate::ledger::EventLedger;
use crate::projector::{memory::MemoryGraph, rebuild_graph_for_tenant};
use crate::queries::get_decision;
use crate::util::require_agent_actor_id;
use crate::Result;

use super::super::backend_error::storage_error;
use super::{replay_events_sql, SqliteEventLedger, SCHEMA_VERSION};

#[test]
fn append_assigns_monotonic_ids() -> Result<()> {
    with_sqlite_ledger("append-monotonic", assert_monotonic_append)
}

#[test]
fn append_is_idempotent_for_duplicate_event_uuid() -> Result<()> {
    with_sqlite_ledger("append-dedup", assert_dedup_by_event_uuid)
}

#[test]
fn replay_from_zero_is_ordered() -> Result<()> {
    with_sqlite_ledger("replay-ordered", |ledger| {
        assert_replay_from_zero_in_order(ledger)
    })
}

#[test]
fn read_applies_offset_and_limit() -> Result<()> {
    with_sqlite_ledger("read-offset-limit", |ledger| {
        assert_read_offset_and_limit(ledger)
    })
}

#[test]
fn grounding_events_round_trip() -> Result<()> {
    with_sqlite_ledger("grounding-round-trip", assert_grounding_events_round_trip)
}

#[test]
fn typed_and_id_reads_agree_with_the_contract() -> Result<()> {
    with_sqlite_ledger("typed-id-reads", |ledger| {
        assert_typed_and_id_reads(ledger, &TenantId::local())
    })
}

#[test]
fn uuid_lookup_agrees_with_the_contract() -> Result<()> {
    with_sqlite_ledger("uuid-lookup", |ledger| {
        assert_event_ids_for_uuids(ledger, &TenantId::local())
    })
}

#[test]
fn replay_into_a_non_empty_ledger_agrees_with_the_contract() -> Result<()> {
    with_sqlite_ledger("replay-non-empty", |ledger| {
        assert_replay_into_non_empty_ledger(ledger, &TenantId::local())
    })
}

#[test]
fn tenant_scoped_sqlite_ledger_isolates_commands_and_queries() -> Result<()> {
    with_sqlite_ledger("tenant-isolation", |ledger| {
        let tenant_a = test_tenant_id("tenant:a")?;
        let tenant_b = test_tenant_id("tenant:b")?;

        write_shared_decision(ledger, &tenant_a, "Tenant A decision")?;
        write_shared_decision(ledger, &tenant_b, "Tenant B decision")?;

        let tenant_a_events = ledger.read_for_tenant(&tenant_a, 0, 10)?;
        let tenant_b_events = ledger.read_for_tenant(&tenant_b, 0, 10)?;
        require_event_count("tenant a event count", &tenant_a_events, 2)?;
        require_event_count("tenant b event count", &tenant_b_events, 2)?;
        require_all_events_for_tenant("tenant a events", &tenant_a_events, &tenant_a)?;
        require_all_events_for_tenant("tenant b events", &tenant_b_events, &tenant_b)?;

        let graph_a = MemoryGraph::default();
        rebuild_graph_for_tenant(ledger, &tenant_a, &graph_a)?;
        let decision_a = get_decision(&graph_a, "decision-shared")?
            .data
            .ok_or_else(|| test_failure("tenant a decision exists"))?;
        require_title("tenant a decision", &decision_a.title, "Tenant A decision")?;

        let graph_b = MemoryGraph::default();
        rebuild_graph_for_tenant(ledger, &tenant_b, &graph_b)?;
        let decision_b = get_decision(&graph_b, "decision-shared")?
            .data
            .ok_or_else(|| test_failure("tenant b decision exists"))?;
        require_title("tenant b decision", &decision_b.title, "Tenant B decision")?;

        Ok(())
    })
}

// `require_agent_actor_id` gates `SqliteUserStore::mint_agent_token` (and its Postgres
// twin); its unit tests live in this UBS-ignored module rather than inline in `util.rs`
// so the assertion-heavy tests do not count against the production warning baseline.
#[test]
fn require_agent_actor_id_accepts_well_shaped_id() {
    assert!(require_agent_actor_id("agent:claude:gastown-crew").is_ok());
}

#[test]
fn require_agent_actor_id_rejects_human_shaped_id() {
    assert!(require_agent_actor_id("human:alex@example.com").is_err());
}

#[test]
fn require_agent_actor_id_rejects_missing_name() {
    assert!(require_agent_actor_id("agent:claude").is_err());
    assert!(require_agent_actor_id("agent:claude:").is_err());
}

#[test]
fn require_agent_actor_id_rejects_missing_tool() {
    assert!(require_agent_actor_id("agent::hook").is_err());
}

#[test]
fn require_agent_actor_id_rejects_bare_uuid() {
    assert!(require_agent_actor_id("550e8400-e29b-41d4-a716-446655440000").is_err());
}

#[test]
fn require_agent_actor_id_accepts_uuid_shaped_name_segment() {
    assert!(require_agent_actor_id("agent:claude:550e8400-e29b-41d4-a716-446655440000").is_ok());
}

#[test]
fn uses_wal_and_creates_file() -> Result<()> {
    with_sqlite_ledger("wal-and-file", |ledger| {
        assert!(ledger.path().exists());

        let journal_mode: Option<String> = ledger
            .connection
            .query_row("PRAGMA journal_mode;", [], |row| row.get(0))
            .optional()
            .map_err(storage_error)?;

        assert_eq!(journal_mode.as_deref(), Some("wal"));
        Ok(())
    })
}

#[test]
fn configures_busy_timeout_for_write_contention() -> Result<()> {
    with_sqlite_ledger("busy-timeout", |ledger| {
        let busy_timeout_ms: i64 = ledger
            .connection
            .query_row("PRAGMA busy_timeout;", [], |row| row.get(0))
            .map_err(storage_error)?;

        assert_eq!(busy_timeout_ms, 30_000);
        Ok(())
    })
}

#[test]
fn concurrent_first_open_initializes_schema_once() -> Result<()> {
    let dir = temp_hivemind_dir("concurrent-first-open");
    fs::create_dir_all(&dir).map_err(storage_error)?;

    const EVENT_IDS: [&str; 8] = [
        "concurrent-first-open-0",
        "concurrent-first-open-1",
        "concurrent-first-open-2",
        "concurrent-first-open-3",
        "concurrent-first-open-4",
        "concurrent-first-open-5",
        "concurrent-first-open-6",
        "concurrent-first-open-7",
    ];
    let worker_count = EVENT_IDS.len();
    let dir = Arc::new(dir);
    let barrier = Arc::new(Barrier::new(worker_count));
    let mut handles = Vec::new();
    for event_id in EVENT_IDS {
        let dir = Arc::clone(&dir);
        let barrier = Arc::clone(&barrier);
        handles.push(thread::spawn(move || -> Result<()> {
            barrier.wait();
            let ledger = SqliteEventLedger::open(dir.as_ref())?;
            ledger.append(make_event(event_id, Uuid::new_v4()))?;
            Ok(())
        }));
    }

    for handle in handles {
        handle
            .join()
            .map_err(|_| storage_error("concurrent open worker panicked"))??;
    }

    let ledger = SqliteEventLedger::open(dir.as_ref())?;
    let expected_events =
        u64::try_from(worker_count).map_err(|_| storage_error("worker count out of range"))?;
    if ledger.latest_offset()? != expected_events {
        return Err(storage_error("concurrent first open event count mismatch").into());
    }

    let _ = fs::remove_dir_all(dir.as_ref());
    Ok(())
}

#[test]
fn opening_an_initialized_ledger_does_not_wait_for_the_write_lock() -> Result<()> {
    let dir = temp_hivemind_dir("open-without-write-lock");
    drop(SqliteEventLedger::open(&dir)?);

    // Another connection sits in a write transaction, as a running append does.
    let writer = rusqlite::Connection::open(dir.join("ledger.sqlite")).map_err(storage_error)?;
    writer
        .execute_batch("BEGIN IMMEDIATE")
        .map_err(storage_error)?;

    let (sender, receiver) = mpsc::channel();
    let open_dir = dir.clone();
    thread::spawn(move || {
        let opened = SqliteEventLedger::open_for_tenant(&open_dir, &TenantId::local()).map(|_| ());
        let _ = sender.send(opened);
    });
    let opened = receiver.recv_timeout(Duration::from_secs(5));

    writer.execute_batch("ROLLBACK").map_err(storage_error)?;
    let _ = fs::remove_dir_all(&dir);
    opened.map_err(|_| storage_error("open waited on the write lock another connection holds"))?
}

#[test]
fn first_open_stamps_the_schema_version_so_later_opens_skip_setup() -> Result<()> {
    with_sqlite_ledger("schema-version-stamp", |ledger| {
        let stamped: i64 = ledger
            .connection
            .query_row("PRAGMA user_version;", [], |row| row.get(0))
            .map_err(storage_error)?;
        assert_eq!(stamped, SCHEMA_VERSION);
        Ok(())
    })
}

#[test]
fn opening_a_ledger_from_before_the_schema_stamp_migrates_it_and_keeps_its_events() -> Result<()> {
    let dir = temp_hivemind_dir("pre-stamp-migration");
    fs::create_dir_all(&dir).map_err(storage_error)?;
    let legacy = rusqlite::Connection::open(dir.join("ledger.sqlite")).map_err(storage_error)?;
    legacy
        .execute_batch(
            "CREATE TABLE events (
                 event_id INTEGER PRIMARY KEY AUTOINCREMENT,
                 event_uuid TEXT NOT NULL UNIQUE,
                 type TEXT NOT NULL,
                 actor_id TEXT NOT NULL,
                 correlation_id TEXT,
                 causation_event_id INTEGER,
                 payload TEXT NOT NULL,
                 ts TEXT NOT NULL
             );
             INSERT INTO events (event_uuid, type, actor_id, payload, ts)
             VALUES ('legacy-uuid', 'evidence.recorded', 'actor:test', '{}', '2026-01-01T00:00:00Z');",
        )
        .map_err(storage_error)?;
    drop(legacy);

    let ledger = SqliteEventLedger::open(&dir)?;
    let kept: i64 = ledger
        .connection
        .query_row(
            "SELECT COUNT(*) FROM events WHERE tenant_id = 'local' AND event_uuid = 'legacy-uuid'",
            [],
            |row| row.get(0),
        )
        .map_err(storage_error)?;
    let stamped: i64 = ledger
        .connection
        .query_row("PRAGMA user_version;", [], |row| row.get(0))
        .map_err(storage_error)?;
    let _ = fs::remove_dir_all(&dir);

    assert_eq!(kept, 1);
    assert_eq!(stamped, SCHEMA_VERSION);
    Ok(())
}

#[test]
fn a_read_after_an_event_id_searches_the_tenant_index_instead_of_scanning_the_tenant() -> Result<()>
{
    with_sqlite_ledger("event-id-index-plan", |ledger| {
        for sql in [
            replay_events_sql(),
            "SELECT COALESCE(MAX(event_id), 0) FROM events WHERE tenant_id = ?1".to_owned(),
        ] {
            let plan = query_plan(ledger, &sql)?;
            assert!(
                plan.contains("events_tenant_event_idx"),
                "`{sql}` should use the (tenant_id, event_id) index: {plan}"
            );
            assert!(
                !plan.contains("SCAN") && !plan.contains("TEMP B-TREE"),
                "`{sql}` should search, not scan and sort: {plan}"
            );
        }
        Ok(())
    })
}

#[test]
fn opening_a_ledger_stamped_before_the_event_id_index_adds_it() -> Result<()> {
    let dir = temp_hivemind_dir("event-id-index-migration");
    let ledger = SqliteEventLedger::open(&dir)?;
    ledger
        .connection
        .execute_batch("DROP INDEX events_tenant_event_idx; PRAGMA user_version = 1;")
        .map_err(storage_error)?;
    drop(ledger);

    let ledger = SqliteEventLedger::open(&dir)?;
    let indexes: i64 = ledger
        .connection
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master
             WHERE type = 'index' AND name = 'events_tenant_event_idx'",
            [],
            |row| row.get(0),
        )
        .map_err(storage_error)?;
    let stamped: i64 = ledger
        .connection
        .query_row("PRAGMA user_version;", [], |row| row.get(0))
        .map_err(storage_error)?;
    let _ = fs::remove_dir_all(&dir);

    assert_eq!(indexes, 1);
    assert_eq!(stamped, SCHEMA_VERSION);
    Ok(())
}

/// `EXPLAIN QUERY PLAN` of `sql` with its parameters unbound, one line per plan step.
fn query_plan(ledger: &SqliteEventLedger, sql: &str) -> Result<String> {
    let mut statement = ledger
        .connection
        .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
        .map_err(storage_error)?;
    let steps = statement
        .query_map([], |row| row.get::<_, String>(3))
        .map_err(storage_error)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(storage_error)?;
    Ok(steps.join("; "))
}

#[test]
#[ignore = "performance benchmark; run in isolated environment"]
fn ten_k_append_plus_read_stays_fast() -> Result<()> {
    with_sqlite_ledger("ten-k-fast", |ledger| {
        let start = Instant::now();

        for index in 0..10_000 {
            let uuid = Uuid::new_v4();
            let event = make_event(&format!("evidence-{index}"), uuid);
            ledger.append(event)?;
        }

        let events = ledger.read(0, 10_000)?;
        assert_eq!(events.len(), 10_000);
        assert!(start.elapsed().as_secs_f64() < 1.0);

        Ok(())
    })
}

fn with_sqlite_ledger<T>(
    prefix: &str,
    f: impl FnOnce(&SqliteEventLedger) -> Result<T>,
) -> Result<T> {
    let dir = temp_hivemind_dir(prefix);
    let ledger = SqliteEventLedger::open(&dir)?;
    let result = f(&ledger);
    let _ = fs::remove_dir_all(&dir);
    result
}

fn temp_hivemind_dir(prefix: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock before unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("hivemind-{prefix}-{nanos}-{}", std::process::id()))
}

fn write_shared_decision(
    ledger: &SqliteEventLedger,
    tenant_id: &TenantId,
    title: &str,
) -> Result<()> {
    let commands = Commands::new_with_context(
        ledger,
        CommandContext::new(
            tenant_id.clone(),
            EventProvenance::api(Some("tenant-test".to_owned())),
        ),
    );
    commands.record_option_with_id(
        "actor:test",
        "option-shared",
        "Shared",
        "Shared option label across tenants",
    )?;
    commands.propose_decision_with_id(
        DecisionProposalInput {
            grounding: Grounding::NotAsked,
            expressed_confidence: None,
            project: None,
            actor_id: "actor:test",
            title,
            rationale: "Tenant-specific rationale for the isolation test",
            topic_keys: &["tenant-isolation".to_owned()],
            option_ids: &["option-shared".to_owned()],
            option_labels: &["Shared".to_owned()],
            chosen_option_id: None,
            decided_by: None,
            delegated_by: None,
            still_proposed: false,
            hypothesis_ids: &[],
            evidence_ids: &[],
            quote: None,
            question: None,
            floor: FloorInput::NotAsked,
        },
        "decision-shared",
        DecisionProposalEventUuids {
            proposal: Uuid::from_u128(1),
            has_option: vec![Uuid::from_u128(2)],
            chose: None,
            assumes: Vec::new(),
            based_on: Vec::new(),
            follows_from: Vec::new(),
        },
    )?;
    Ok(())
}

fn test_tenant_id(value: &str) -> Result<TenantId> {
    TenantId::new(value)
        .map_err(|error| test_failure(format!("invalid test tenant id {value}: {error}")))
}

fn require_event_count(label: &str, events: &[Event], expected: usize) -> Result<()> {
    if events.len() == expected {
        Ok(())
    } else {
        Err(test_failure(format!(
            "{label}: expected {expected}, got {}",
            events.len()
        )))
    }
}

fn require_all_events_for_tenant(
    label: &str,
    events: &[Event],
    tenant_id: &TenantId,
) -> Result<()> {
    if events.iter().all(|event| event.tenant_id == *tenant_id) {
        Ok(())
    } else {
        Err(test_failure(format!(
            "{label}: expected only tenant {tenant_id}"
        )))
    }
}

fn require_title(label: &str, actual: &str, expected: &str) -> Result<()> {
    if actual == expected {
        Ok(())
    } else {
        Err(test_failure(format!(
            "{label}: expected title {expected:?}, got {actual:?}"
        )))
    }
}

fn test_failure(message: impl Into<String>) -> crate::HivemindError {
    CommandError::Invariant(message.into()).into()
}
