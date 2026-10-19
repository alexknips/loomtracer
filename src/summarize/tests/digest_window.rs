//! A digest of a window larger than one page keeps the newest decisions and pages the rest
//! (hivemind-4focm).

use super::*;
use crate::ledger::{EventLedger, SqliteEventLedger};
use crate::queries::test_fixtures::{ts, Scenario};
use crate::Result;

/// A window copied into a SQLite ledger, so the digest takes the FTS path a real install takes.
struct Window {
    _dir: tempfile::TempDir,
    ledger: AnyLedger,
    graph: MemoryGraph,
}

fn window_of(scenario: &Scenario) -> Result<Window> {
    let dir = tempfile::tempdir().map_err(|error| {
        crate::QueryError::Execution(format!("temp dir for the sqlite ledger: {error}"))
    })?;
    let sqlite = SqliteEventLedger::open(dir.path())?;
    scenario.ledger().replay_from(0, &mut |event| {
        sqlite.append(event.clone())?;
        Ok(())
    })?;
    let graph = MemoryGraph::default();
    rebuild_graph_for_tenant(&sqlite, &TenantId::local(), &graph)?;
    Ok(Window {
        _dir: dir,
        ledger: AnyLedger::Sqlite(sqlite),
        graph,
    })
}

/// 60 decisions decided in February 2026, and the ids of them newest decided first: 55
/// proposals (`decision-00` is the oldest, at 10:00 on the 10th, `decision-54` the newest, 54
/// minutes later) and 5 classifier drafts (`capture:<event>:0`, which sort ahead of every
/// `decision-` id). Two drafts were said after every proposal and three before every one of
/// them, all classified after all of it: a draft is dated by when it was said.
fn sixty_decisions() -> Result<(Window, Vec<String>)> {
    let scenario = Scenario::new();
    for minute in 0..55 {
        scenario.decision(
            &format!("decision-{minute:02}"),
            &format!("Proposal {minute}"),
            "human:alice",
            &format!("2026-02-10T10:{minute:02}:00Z"),
        )?;
    }
    let mut said_after = Vec::new();
    for said_at in ["2026-02-20T09:00:00Z", "2026-02-20T09:30:00Z"] {
        said_after.push(scenario.classified_decision_said_at(
            "agent:claude:classifier",
            "2026-02-25T09:00:00Z",
            &format!("Said at {said_at}"),
            None,
            Some(said_at),
        )?);
    }
    let mut said_before = Vec::new();
    for said_at in [
        "2026-02-01T08:00:00Z",
        "2026-02-01T08:10:00Z",
        "2026-02-01T08:20:00Z",
    ] {
        said_before.push(scenario.classified_decision_said_at(
            "agent:claude:classifier",
            "2026-02-25T09:00:00Z",
            &format!("Said at {said_at}"),
            None,
            Some(said_at),
        )?);
    }

    let mut newest_first: Vec<String> = said_after.into_iter().rev().collect();
    newest_first.extend((0..55).rev().map(|minute| format!("decision-{minute:02}")));
    newest_first.extend(said_before.into_iter().rev());
    Ok((window_of(&scenario)?, newest_first))
}

fn digest(window: &Window, limit: usize, cursor: Option<&str>) -> Result<DigestResponse> {
    let request = DigestRequest {
        since: ts("2026-01-01T00:00:00Z"),
        until: ts("2026-03-01T00:00:00Z"),
        actor_ids: Vec::new(),
        limit,
        cursor: cursor.map(str::to_owned),
    };
    Ok(weekly_digest(
        &QueryContext::local(),
        &window.ledger,
        &window.graph,
        &request,
    )?
    .data)
}

fn ids(response: &DigestResponse) -> Vec<&str> {
    response
        .entries
        .iter()
        .map(|entry| entry.decision_id.as_str())
        .collect()
}

#[test]
fn a_window_of_sixty_shows_the_newest_fifty_and_the_other_ten_by_cursor() -> Result<()> {
    let (window, newest_first) = sixty_decisions()?;

    let first = digest(&window, DIGEST_MAX_DECISIONS, None)?;
    assert_eq!(ids(&first), &newest_first[..50]);
    assert_eq!(first.total_in_window, 60);
    assert!(first.truncated);
    assert_eq!(first.next_cursor.as_deref(), Some("50"));
    assert!(
        first.text.contains(
            "Note: showing decisions 1-50 of 60 in this window, newest first by when they were \
             decided. Read the rest with --cursor 50."
        ),
        "{}",
        first.text
    );
    assert!(
        !first.text.contains("--limit"),
        "the note must not point at a cap that is already reached: {}",
        first.text
    );
    let times: Vec<_> = first.entries.iter().map(|entry| entry.decided_at).collect();
    assert!(
        times.iter().all(Option::is_some) && times.windows(2).all(|pair| pair[0] >= pair[1]),
        "entries must say when they were decided, newest first: {times:?}"
    );

    let rest = digest(&window, DIGEST_MAX_DECISIONS, first.next_cursor.as_deref())?;
    assert_eq!(ids(&rest), &newest_first[50..]);
    assert_eq!(rest.total_in_window, 60);
    assert!(!rest.truncated);
    assert_eq!(rest.next_cursor, None);
    assert!(
        rest.text.contains(
            "Note: showing decisions 51-60 of 60 in this window, newest first by when they were \
             decided."
        ),
        "{}",
        rest.text
    );
    assert!(!rest.text.contains("--cursor"), "{}", rest.text);
    Ok(())
}

#[test]
fn a_smaller_limit_walks_the_whole_window_once() -> Result<()> {
    let (window, newest_first) = sixty_decisions()?;

    let mut seen: Vec<String> = Vec::new();
    let mut cursors = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let page = digest(&window, 25, cursor.as_deref())?;
        seen.extend(page.entries.iter().map(|entry| entry.decision_id.clone()));
        cursors.push(page.next_cursor.clone());
        match page.next_cursor {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }

    assert_eq!(seen, newest_first);
    assert_eq!(
        cursors,
        vec![Some("25".to_owned()), Some("50".to_owned()), None]
    );
    Ok(())
}

#[test]
fn decisions_decided_at_the_same_moment_come_in_id_order_and_a_window_that_fits_has_no_note(
) -> Result<()> {
    let scenario = Scenario::new();
    for id in ["decision-c", "decision-a", "decision-b"] {
        scenario.decision(id, id, "human:alice", "2026-02-10T10:00:00Z")?;
    }
    scenario.decision(
        "decision-z",
        "decision-z",
        "human:alice",
        "2026-02-11T10:00:00Z",
    )?;
    let window = window_of(&scenario)?;

    let digest = digest(&window, DIGEST_MAX_DECISIONS, None)?;

    assert_eq!(
        ids(&digest),
        ["decision-z", "decision-a", "decision-b", "decision-c"]
    );
    assert!(!digest.truncated);
    assert_eq!(digest.next_cursor, None);
    assert!(!digest.text.contains("Note:"), "{}", digest.text);
    Ok(())
}

#[test]
fn a_cursor_that_is_not_an_offset_into_the_window_is_refused() -> Result<()> {
    let (window, _) = sixty_decisions()?;

    let past_the_end = digest(&window, DIGEST_MAX_DECISIONS, Some("60"))
        .expect_err("a cursor at the end of the window reads nothing");
    assert!(
        past_the_end
            .to_string()
            .contains("cursor 60 is past the end of the window, which holds 60 decision(s)"),
        "{past_the_end}"
    );
    let not_a_number =
        digest(&window, DIGEST_MAX_DECISIONS, Some("newest")).expect_err("a cursor is a number");
    assert!(
        not_a_number
            .to_string()
            .contains("cursor must be a non-negative offset"),
        "{not_a_number}"
    );
    Ok(())
}
