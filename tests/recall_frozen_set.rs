//! The frozen recall question set (`tests/fixtures/recall_frozen/questions.tsv`, run by
//! `scripts/recall_frozen_set.py`) is how a build is judged better or worse on the two kinds of
//! wrong recall answer, NEARBY and SYNONYM (hivemind-y7b6c), and `strays.tsv` beside it on the
//! third, a question about nothing the ledger holds that finds a decision (STRAY,
//! hivemind-k25y8). Two runs only compare when they ask the same questions, so this pins both
//! files: a change to a row, a kind or the row count fails here and says to start a new file
//! instead of editing this one.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

const FIXTURE: &str = "tests/fixtures/recall_frozen/questions.tsv";
const STRAYS: &str = "tests/fixtures/recall_frozen/strays.tsv";

/// The rows the bead names (CB4, BQ4, H1, K3, R4, V2, the beadline question, the coffee control,
/// the backups control, BB2) must stay in the set.
const NAMED_ROWS: &[&str] = &[
    "checker:CB4",
    "checker:BQ4",
    "checker:H1",
    "checker:K3",
    "checker:R4",
    "checker:V2",
    "checker:BB2",
    "pobj2:PB1",
    "pen9l:C2",
    "fresh:N12",
];

struct Row {
    id: String,
    kind: String,
    expect: String,
    role: String,
}

fn data_lines(fixture: &str) -> Vec<String> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(fixture);
    let text = fs::read_to_string(&path).unwrap_or_else(|err| panic!("{}: {err}", path.display()));
    text.lines()
        .filter(|line| !line.trim().is_empty() && !line.starts_with('#'))
        .map(str::to_owned)
        .collect()
}

fn rows() -> Vec<Row> {
    [FIXTURE, STRAYS]
        .into_iter()
        .flat_map(data_lines)
        .collect::<Vec<_>>()
        .iter()
        .map(|line| {
            let cols: Vec<&str> = line.split('\t').collect();
            assert_eq!(cols.len(), 5, "id, kind, expect, question, role: {line}");
            assert!(!cols[3].trim().is_empty(), "a row asks a question: {line}");
            Row {
                id: cols[0].to_owned(),
                kind: cols[1].to_owned(),
                expect: cols[2].to_owned(),
                role: cols[4].to_owned(),
            }
        })
        .collect()
}

/// FNV-1a over the data rows, each with its newline: the fixture's identity.
fn fingerprint(fixture: &str) -> u64 {
    data_lines(fixture)
        .iter()
        .flat_map(|line| line.bytes().chain(std::iter::once(b'\n')))
        .fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
        })
}

#[test]
fn every_row_names_a_kind_a_role_and_the_decision_it_expects() {
    let mut ids = BTreeSet::new();
    for row in rows() {
        assert!(ids.insert(row.id.clone()), "{} appears twice", row.id);
        assert!(
            matches!(row.kind.as_str(), "NEARBY" | "SYNONYM" | "STRAY"),
            "{}: kind {:?}",
            row.id,
            row.kind
        );
        assert!(
            matches!(row.role.as_str(), "fail" | "guard"),
            "{}: role {:?}",
            row.id,
            row.role
        );
        let prefixes_ok = row.expect.split('|').all(|prefix| {
            prefix == "none" || (prefix.len() == 8 && prefix.bytes().all(|b| b.is_ascii_hexdigit()))
        });
        assert!(prefixes_ok, "{}: expect {:?}", row.id, row.expect);
        if row.kind == "SYNONYM" {
            assert_ne!(
                row.expect, "none",
                "{}: a synonym row has a decision to miss",
                row.id
            );
        }
        if row.kind == "STRAY" {
            assert_eq!(
                row.expect, "none",
                "{}: a stray row asks about nothing the ledger holds",
                row.id
            );
        }
    }
}

#[test]
fn the_set_holds_the_rows_the_bead_names_and_all_three_kinds() {
    let rows = rows();
    let ids: BTreeSet<&str> = rows.iter().map(|row| row.id.as_str()).collect();
    for named in NAMED_ROWS {
        assert!(ids.contains(named), "{named} is missing from the set");
    }
    let mut counts: BTreeMap<(&str, &str), usize> = BTreeMap::new();
    for row in &rows {
        *counts
            .entry((row.kind.as_str(), row.role.as_str()))
            .or_default() += 1;
    }
    assert_eq!(
        counts,
        BTreeMap::from([
            (("NEARBY", "fail"), 123),
            (("NEARBY", "guard"), 59),
            (("STRAY", "fail"), 3),
            (("STRAY", "guard"), 46),
            (("SYNONYM", "fail"), 94),
        ]),
        "the set changed: start a new file instead"
    );
}

#[test]
fn the_set_is_the_one_that_was_frozen() {
    for (fixture, pinned) in [
        (FIXTURE, "0xeff30fed9627fb7a"),
        (STRAYS, "0xb4714603b5a392e3"),
    ] {
        assert_eq!(
            format!("{:#018x}", fingerprint(fixture)),
            pinned,
            "a row of {fixture} changed, so two builds' counts no longer compare. A new question \
             set is a new file; this one stays as it was."
        );
    }
}
