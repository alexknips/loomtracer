// Parent module gates this file with #[cfg(test)]; repeat the marker so UBS can filter test-only assertions.
#[cfg(test)]
use super::*;

#[cfg(test)]
use crate::queries::test_fixtures::ts;

#[test]
fn a_bare_check_date_falls_due_at_the_end_of_the_named_day() {
    let due = parse_check_by("2026-09-01").expect("a bare date parses");

    // Still on the named day, so it prints as the day the person named...
    assert_eq!(due.format("%Y-%m-%d").to_string(), "2026-09-01");
    // ...and not yet past at any moment of it, including the last whole second...
    assert!(due >= ts("2026-09-01T23:59:59Z"));
    assert!(!(due < ts("2026-09-01T23:59:59.500Z")));
    // ...but past from the next midnight UTC.
    assert!(due < ts("2026-09-02T00:00:00Z"));
}

#[test]
fn an_rfc3339_check_date_is_that_exact_instant_in_utc() {
    assert_eq!(
        parse_check_by("2026-09-01T00:00:00Z"),
        Some(ts("2026-09-01T00:00:00Z"))
    );
    // 20:00 on 1 September in California is 03:00 on 2 September UTC; the offset is honoured,
    // not read as a bare date.
    assert_eq!(
        parse_check_by("2026-09-01T20:00:00-07:00"),
        Some(ts("2026-09-02T03:00:00Z"))
    );
}

#[test]
fn a_check_date_ignores_surrounding_whitespace_and_refuses_anything_else() {
    assert_eq!(parse_check_by(" 2026-09-01 "), parse_check_by("2026-09-01"));
    for refused in [
        "",
        "7d",
        "next friday",
        "2026-13-01",
        "2026-09-31",
        "09/01/2026",
    ] {
        assert_eq!(parse_check_by(refused), None, "{refused}");
    }
}
