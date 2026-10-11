// Parent module gates this file with #[cfg(test)]; repeat the marker so UBS can filter test-only assertions.
#[cfg(test)]
use serde_json::json;

use super::*;

fn wire(binds: Value, undo_cost: Value) -> std::result::Result<FloorAnswers, String> {
    FloorAnswers::from_wire(Some(&binds), Some(&undo_cost))
}

#[test]
fn the_floor_clears_when_a_decision_binds_something_or_is_costly_to_undo() {
    use BindsLevel::{Nothing, Something};
    use FloorVerdict::{Below, Cleared};
    use UndoCostLevel::{Cheap, Costly};

    // The plan's table: apple-vs-Apple is below, a docs rule binds, a rename binds, merging
    // tonight despite the flaky test is costly to undo.
    assert_eq!(floor_verdict(Some(Nothing), Some(Cheap)), Below);
    assert_eq!(floor_verdict(Some(Something), Some(Cheap)), Cleared);
    assert_eq!(floor_verdict(Some(Something), Some(Costly)), Cleared);
    assert_eq!(floor_verdict(Some(Nothing), Some(Costly)), Cleared);
}

#[test]
fn a_decision_is_not_asked_unless_both_answers_exist() {
    use FloorVerdict::NotAsked;

    assert_eq!(floor_verdict(None, None), NotAsked);
    assert_eq!(floor_verdict(Some(BindsLevel::Something), None), NotAsked);
    assert_eq!(floor_verdict(None, Some(UndoCostLevel::Costly)), NotAsked);
}

#[test]
fn the_stored_levels_read_back_to_the_same_verdict_and_unknown_text_reads_as_not_asked() {
    assert_eq!(
        floor_verdict_from_stored(Some("nothing"), Some("cheap")),
        FloorVerdict::Below
    );
    assert_eq!(
        floor_verdict_from_stored(Some("something"), Some("cheap")),
        FloorVerdict::Cleared
    );
    assert_eq!(
        floor_verdict_from_stored(Some("nothing"), Some("costly")),
        FloorVerdict::Cleared
    );
    assert_eq!(
        floor_verdict_from_stored(Some("nothing"), None),
        FloorVerdict::NotAsked
    );
    assert_eq!(
        floor_verdict_from_stored(Some("a lot"), Some("cheap")),
        FloorVerdict::NotAsked
    );
}

#[test]
fn from_wire_parses_the_four_level_combinations() {
    let answers = wire(
        json!({"level": "something", "statement": "Every script that calls the old name"}),
        json!({"level": "costly", "statement": "A migration of the stored ids"}),
    )
    .expect("a stated pair parses");
    assert_eq!(
        answers.binds,
        BindsAnswer::something("Every script that calls the old name")
    );
    assert_eq!(
        answers.undo_cost,
        UndoCostAnswer::costly("A migration of the stored ids")
    );
    assert_eq!(answers.verdict(), FloorVerdict::Cleared);

    let answers = wire(json!({"level": "nothing"}), json!({"level": "cheap"}))
        .expect("the two zero answers parse");
    assert_eq!(answers.binds, BindsAnswer::nothing());
    assert_eq!(answers.undo_cost, UndoCostAnswer::cheap());
    assert_eq!(answers.verdict(), FloorVerdict::Below);
}

#[test]
fn from_wire_refuses_a_missing_answer_naming_both_questions() {
    for (binds, undo_cost) in [
        (None, None),
        (Some(json!({"level": "nothing"})), None),
        (None, Some(json!({"level": "cheap"}))),
        (Some(Value::Null), Some(json!({"level": "cheap"}))),
        (Some(json!({"level": "nothing"})), Some(Value::Null)),
    ] {
        let refusal = FloorAnswers::from_wire(binds.as_ref(), undo_cost.as_ref())
            .expect_err("a capture that leaves an answer out is refused");
        assert_eq!(refusal, FLOOR_REFUSAL);
    }
    assert!(FLOOR_REFUSAL.contains("binds") && FLOOR_REFUSAL.contains("undo cost"));
    assert!(FLOOR_REFUSAL.contains("nothing") && FLOOR_REFUSAL.contains("cheap"));
    assert!(FLOOR_REFUSAL.contains("nothing was written"));
}

#[test]
fn from_wire_refuses_malformed_answers_naming_the_field() {
    let cheap = json!({"level": "cheap"});
    let nothing = json!({"level": "nothing"});
    for (binds, undo_cost, expected) in [
        (
            json!({"level": "something"}),
            cheap.clone(),
            "`binds` at level `something` needs a `statement`",
        ),
        (
            json!({"level": "something", "statement": "   "}),
            cheap.clone(),
            "`binds.statement` must not be blank",
        ),
        (
            json!({"level": "nothing", "statement": "but it does"}),
            cheap.clone(),
            "`binds` at level `nothing` takes no `statement`",
        ),
        (
            nothing.clone(),
            json!({"level": "costly"}),
            "`undo_cost` at level `costly` needs a `statement`",
        ),
        (
            nothing.clone(),
            json!({"level": "cheap", "statement": "it is not"}),
            "`undo_cost` at level `cheap` takes no `statement`",
        ),
        (
            json!({"level": "plenty"}),
            cheap.clone(),
            "`binds.level` `plenty` is not one of nothing, something",
        ),
        (
            nothing.clone(),
            json!({"level": "free"}),
            "`undo_cost.level` `free` is not one of cheap, costly",
        ),
        (
            json!({}),
            cheap.clone(),
            "`binds.level` must be one of nothing, something",
        ),
        (json!("nothing"), cheap.clone(), "`binds` must be an object"),
        (
            json!({"level": "nothing", "why": "x"}),
            cheap.clone(),
            "`binds` has unknown field `why`",
        ),
        (
            nothing.clone(),
            json!({"level": "cheap", "cost": 1}),
            "`undo_cost` has unknown field `cost`",
        ),
        (
            json!({"level": "something", "statement": 7}),
            cheap.clone(),
            "`binds.statement` must be a string",
        ),
    ] {
        let refusal =
            wire(binds.clone(), undo_cost.clone()).expect_err("a malformed answer is refused");
        assert!(
            refusal.contains(expected),
            "{binds} / {undo_cost} -> {refusal}, wanted {expected}"
        );
    }
}

#[test]
fn the_reply_names_both_levels_and_marks_only_a_below_floor_capture() {
    let cleared = wire(
        json!({"level": "something", "statement": "Other teams' scripts"}),
        json!({"level": "cheap"}),
    )
    .expect("parses")
    .reply();
    assert!(cleared.cleared);
    assert_eq!(cleared.below_floor(), None);
    assert_eq!(cleared.note(), None);
    let mut reply = json!({"decision_id": "decision-1"});
    cleared.insert_into(&mut reply);
    assert_eq!(
        reply,
        json!({
            "decision_id": "decision-1",
            "floor": {"binds": "something", "undo_cost": "cheap", "cleared": true}
        })
    );

    let below = wire(json!({"level": "nothing"}), json!({"level": "cheap"}))
        .expect("parses")
        .reply();
    assert!(!below.cleared);
    assert_eq!(below.below_floor(), Some(true));
    assert_eq!(below.note(), Some(BELOW_FLOOR_NOTE));
    let mut reply = json!({"decision_id": "decision-2"});
    below.insert_into(&mut reply);
    assert_eq!(
        reply,
        json!({
            "decision_id": "decision-2",
            "floor": {"binds": "nothing", "undo_cost": "cheap", "cleared": false},
            "below_floor": true,
            "below_floor_note": BELOW_FLOOR_NOTE
        })
    );
}

#[test]
fn an_answer_serializes_as_level_and_statement_and_drops_an_absent_statement() {
    assert_eq!(
        serde_json::to_value(BindsAnswer::nothing()).expect("serializes"),
        json!({"level": "nothing"})
    );
    assert_eq!(
        serde_json::to_value(UndoCostAnswer::costly("Redo the work")).expect("serializes"),
        json!({"level": "costly", "statement": "Redo the work"})
    );
    let parsed: BindsAnswer =
        serde_json::from_value(json!({"level": "something", "statement": "Later docs"}))
            .expect("round trips");
    assert_eq!(parsed, BindsAnswer::something("Later docs"));
    assert!(
        serde_json::from_value::<BindsAnswer>(json!({"level": "nothing", "extra": true})).is_err(),
        "an unknown field is refused"
    );
}

// ── What a read says of a recorded decision (hivemind-bbnw.11) ──

#[test]
fn a_recorded_floor_carries_both_answers_and_the_verdict_they_reach() {
    let rename = RecordedFloor::from_stored(
        Some("something"),
        Some("Users and scripts that call the old name"),
        Some("cheap"),
        None,
    );
    assert_eq!(rename.verdict, FloorVerdict::Cleared);
    assert_eq!(
        rename.binds,
        Some(BindsAnswer::something(
            "Users and scripts that call the old name"
        ))
    );
    assert_eq!(rename.undo_cost, Some(UndoCostAnswer::cheap()));
    assert!(!rename.is_below());
    assert_eq!(rename.below_floor_line(), None);

    let apple = RecordedFloor::from_stored(Some("nothing"), None, Some("cheap"), None);
    assert_eq!(apple.verdict, FloorVerdict::Below);
    assert!(apple.is_below());
    assert_eq!(
        apple.below_floor_line().as_deref(),
        Some("floor: below the decision floor")
    );

    let merge_tonight = RecordedFloor::from_stored(
        Some("nothing"),
        None,
        Some("costly"),
        Some("An audit will ask why we ignored it"),
    );
    assert_eq!(merge_tonight.verdict, FloorVerdict::Cleared);
}

#[test]
fn the_lines_a_read_prints_say_the_answers_and_not_asked_where_there_are_none() {
    let rename = RecordedFloor::from_stored(
        Some("something"),
        Some("Users and scripts that call the old name"),
        Some("costly"),
        Some("Renaming back breaks every script already moved"),
    );
    assert_eq!(
        rename.binds_line(),
        "binds: something (Users and scripts that call the old name)"
    );
    assert_eq!(
        rename.undo_cost_line(),
        "undo cost: costly (Renaming back breaks every script already moved)"
    );

    let apple = RecordedFloor::from_stored(Some("nothing"), None, Some("cheap"), None);
    assert_eq!(apple.binds_line(), "binds: nothing");
    assert_eq!(apple.undo_cost_line(), "undo cost: cheap");

    // Said, not left blank: a reader must not take silence for "nothing".
    let old = RecordedFloor::not_asked();
    assert_eq!(old.binds_line(), "binds: not asked");
    assert_eq!(old.undo_cost_line(), "undo cost: not asked");
    assert_eq!(old.below_floor_line(), None);
    assert_eq!(
        RecordedFloor::from_stored(None, None, None, None),
        old,
        "a node that stores no answer reads not asked"
    );
}

#[test]
fn a_level_the_reader_does_not_know_reads_as_that_answer_not_asked() {
    // One good answer is not enough for a verdict: the floor needs both.
    let half = RecordedFloor::from_stored(Some("nothing"), None, Some("enormous"), None);
    assert_eq!(half.verdict, FloorVerdict::NotAsked);
    assert_eq!(half.binds_line(), "binds: nothing");
    assert_eq!(half.undo_cost_line(), "undo cost: not asked");
    assert!(
        !half.is_below(),
        "a decision with one unreadable answer is not marked below"
    );
}

#[test]
fn a_recorded_floor_serializes_flat_with_the_verdict_and_no_key_for_an_answer_never_asked() {
    let flat =
        |floor: &RecordedFloor| serde_json::to_value(floor).expect("a recorded floor serializes");
    assert_eq!(
        flat(&RecordedFloor::from_stored(
            Some("something"),
            Some("Later docs"),
            Some("cheap"),
            None
        )),
        json!({
            "binds": {"level": "something", "statement": "Later docs"},
            "undo_cost": {"level": "cheap"},
            "floor": "cleared"
        })
    );
    assert_eq!(
        flat(&RecordedFloor::from_stored(
            Some("nothing"),
            None,
            Some("cheap"),
            None
        )),
        json!({
            "binds": {"level": "nothing"},
            "undo_cost": {"level": "cheap"},
            "floor": "below"
        })
    );
    assert_eq!(
        flat(&RecordedFloor::not_asked()),
        json!({"floor": "not_asked"}),
        "a decision recorded before the answers existed has no answer keys, only the verdict"
    );
}
