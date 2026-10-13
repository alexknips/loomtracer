// Parent module gates this file with #[cfg(test)]; repeat the marker so UBS can filter test-only assertions.
#[cfg(test)]
use crate::queries::test_fixtures::{reexamine_scenario, Scenario};
use crate::queries::{ChangeKind, QueryContext};
use crate::Result;

use super::*;

fn plan(scenario: &Scenario, decision_id: &str, premise: Option<&str>) -> Result<ReaffirmPlan> {
    plan_reaffirmation(
        &QueryContext::local(),
        &scenario.graph()?,
        scenario.ledger(),
        decision_id,
        premise,
    )
}

fn refusal(plan: ReaffirmPlan) -> String {
    match plan {
        ReaffirmPlan::Refused(text) => text,
        other => panic!("expected a refusal, got {other:?}"), // ubs:ignore: test-only; panicking is correct in tests
    }
}

fn target(plan: ReaffirmPlan) -> ReaffirmTarget {
    match plan {
        ReaffirmPlan::Ready(target) => *target,
        other => panic!("expected one open item, got {other:?}"), // ubs:ignore: test-only; panicking is correct in tests
    }
}

/// A decision that rests on two premises, both superseded: two open items.
fn two_open_items() -> Result<Scenario> {
    let s = Scenario::new();
    let crew = "agent:claude:crew";
    for (id, title, at) in [
        ("d:p1", "Cap requests per tenant", "2026-09-01T00:00:00Z"),
        ("d:p1-new", "Cap requests per user", "2026-09-02T00:00:00Z"),
        ("d:p2", "Bill monthly", "2026-09-01T00:01:00Z"),
        ("d:p2-new", "Bill weekly", "2026-09-02T00:01:00Z"),
        ("d:leaf", "Shard by tenant", "2026-09-01T00:02:00Z"),
    ] {
        s.decision(id, title, crew, at)?;
    }
    for premise in ["d:p1", "d:p2"] {
        s.relation(
            "FOLLOWS_FROM",
            "d:leaf",
            premise,
            crew,
            None,
            "2026-09-01T00:03:00Z",
        )?;
    }
    s.supersede("d:p1", "d:p1-new", "human:alex", "2026-09-03T00:00:00Z")?;
    s.supersede("d:p2", "d:p2-new", "human:alex", "2026-09-03T00:00:01Z")?;
    Ok(s)
}

#[test]
fn one_open_item_is_the_one_a_call_with_no_premise_closes() -> Result<()> {
    let scenario = reexamine_scenario()?;

    let ready = target(plan(&scenario, "d:open", None)?);

    assert_eq!(ready.decision_id, "d:open");
    assert_eq!(ready.decision_title, "Shard by tenant");
    assert_eq!(ready.premise_id, "d:p-sup");
    assert_eq!(ready.premise_label, "Cap requests per tenant");
    assert_eq!(ready.premise_kind, PremiseKind::Decision);
    assert_eq!(ready.covers.kind, ChangeKind::Superseded);
    assert_eq!(ready.covers.by_ids, vec!["d:p-sup-new".to_owned()]);
    assert!(
        ready.covers.at_offset.is_some(),
        "the change is named by its offset"
    );
    Ok(())
}

#[test]
fn a_hypothesis_premise_is_closed_the_same_way() -> Result<()> {
    let scenario = reexamine_scenario()?;

    let ready = target(plan(&scenario, "d:on-assumption", None)?);

    assert_eq!(ready.premise_id, "h:assumption");
    assert_eq!(ready.premise_kind, PremiseKind::Hypothesis);
    assert_eq!(ready.covers.kind, ChangeKind::Refuted);
    Ok(())
}

#[test]
fn a_contested_premise_opens_an_item_a_call_can_close() -> Result<()> {
    let scenario = reexamine_scenario()?;

    let ready = target(plan(&scenario, "d:on-cont", None)?);

    assert_eq!(ready.premise_id, "d:p-cont");
    assert_eq!(ready.covers.kind, ChangeKind::Contested);
    Ok(())
}

#[test]
fn a_premise_is_named_by_its_id_or_by_words_of_its_title() -> Result<()> {
    let scenario = reexamine_scenario()?;

    for premise in ["d:p-sup", "PER TENANT", "cap requests"] {
        let ready = target(plan(&scenario, "d:open", Some(premise))?);
        assert_eq!(ready.premise_id, "d:p-sup", "{premise}");
    }
    Ok(())
}

#[test]
fn several_open_items_with_no_premise_named_are_listed_and_nothing_is_picked() -> Result<()> {
    let scenario = two_open_items()?;

    let ReaffirmPlan::Ambiguous(ambiguous) = plan(&scenario, "d:leaf", None)? else {
        panic!("expected the open items to be listed"); // ubs:ignore: test-only; panicking is correct in tests
    };

    let premises: Vec<&str> = ambiguous
        .items
        .iter()
        .map(|item| item.entry.premise_id.as_str())
        .collect();
    assert_eq!(premises, vec!["d:p1", "d:p2"]);
    let envelope = ambiguous.envelope();
    assert_eq!(envelope["data"]["outcome"], "ambiguous");
    assert_eq!(envelope["data"]["field"], "premise");
    assert_eq!(envelope["data"]["decision_id"], "d:leaf");
    assert_eq!(envelope["result_count"], 2);
    Ok(())
}

#[test]
fn naming_one_of_several_open_premises_closes_that_one() -> Result<()> {
    let scenario = two_open_items()?;

    let by_id = target(plan(&scenario, "d:leaf", Some("d:p2"))?);
    let by_words = target(plan(&scenario, "d:leaf", Some("bill"))?);

    assert_eq!(by_id.premise_id, "d:p2");
    assert_eq!(by_words.premise_id, "d:p2");
    Ok(())
}

#[test]
fn words_that_fit_several_open_premises_are_ambiguous_not_a_guess() -> Result<()> {
    let scenario = two_open_items()?;

    // "n" sits in both "Cap requests per tenant" and "Bill monthly".
    let outcome = plan(&scenario, "d:leaf", Some("n"))?;

    assert!(matches!(outcome, ReaffirmPlan::Ambiguous(_)), "{outcome:?}");
    Ok(())
}

#[test]
fn an_item_already_reaffirmed_is_refused_naming_who_closed_it_when_and_with_which_event(
) -> Result<()> {
    let scenario = reexamine_scenario()?;

    for premise in [None, Some("d:p-sup")] {
        let text = refusal(plan(&scenario, "d:twice", premise)?);

        assert!(text.starts_with("nothing to re-examine:"), "{text}");
        assert!(
            text.contains("already reaffirmed after its latest change"),
            "{text}"
        );
        assert!(
            text.contains("human:alice"),
            "the first reaffirmation closed it: {text}"
        );
        assert!(text.contains("2026-09-03 00:00 UTC"), "{text}");
        assert!(
            text.contains("Billing by request count never depended on the per-tenant cap"),
            "{text}"
        );
        assert!(text.contains("(event "), "{text}");
        assert!(text.contains(", id "), "{text}");
        assert!(text.contains("and 1 more time(s) since"), "{text}");
        assert!(text.ends_with("nothing was written"), "{text}");
    }
    Ok(())
}

#[test]
fn the_event_a_refusal_names_is_the_reaffirmation_in_the_ledger() -> Result<()> {
    let scenario = reexamine_scenario()?;
    let graph = scenario.graph()?;
    let items = items_of(&QueryContext::local(), &graph, scenario.ledger(), "d:twice")?;
    let ItemState::Reaffirmed { reaffirmations } = &items[0].entry.state else {
        panic!("d:twice is reaffirmed"); // ubs:ignore: test-only; panicking is correct in tests
    };

    let text = refusal(plan(&scenario, "d:twice", None)?);

    assert!(
        text.contains(&format!("id {}", reaffirmations[0].id)),
        "its uuid: {text}"
    );
    let origin = reaffirmations[0]
        .event_origin
        .expect("the node carries its ledger offset"); // ubs:ignore: test-only; panicking is correct in tests
    assert!(
        text.contains(&format!("(event {origin}, ")),
        "its ledger number: {text}"
    );
    Ok(())
}

#[test]
fn a_dependent_that_stopped_standing_has_nothing_to_re_examine() -> Result<()> {
    let scenario = reexamine_scenario()?;

    let superseded = refusal(plan(&scenario, "d:after-sup", None)?);
    let rejected = refusal(plan(&scenario, "d:after-rej", None)?);

    assert!(
        superseded.contains("was itself superseded (by d:after-sup-v2)"),
        "{superseded}"
    );
    assert!(rejected.contains("was itself rejected"), "{rejected}");
    Ok(())
}

#[test]
fn a_decision_superseded_before_its_premise_changed_is_refused_as_no_longer_standing() -> Result<()>
{
    let scenario = reexamine_scenario()?;

    for premise in [None, Some("d:p-sup")] {
        let text = refusal(plan(&scenario, "d:before-sup", premise)?);

        assert!(
            text.contains("is superseded and no longer stands"),
            "{text}"
        );
    }
    Ok(())
}

#[test]
fn a_premise_that_has_not_changed_is_refused_as_such() -> Result<()> {
    let scenario = reexamine_scenario()?;

    let by_id = refusal(plan(&scenario, "d:on-fine", Some("h:fine"))?);
    let none = refusal(plan(&scenario, "d:on-fine", None)?);

    assert!(
        by_id.contains("\"The API is stable\" has not changed"),
        "{by_id}"
    );
    assert!(none.contains("none of the 1 premise(s)"), "{none}");
    assert!(none.contains("has changed"), "{none}");
    Ok(())
}

#[test]
fn a_decision_that_does_not_rest_on_the_named_premise_says_what_it_rests_on() -> Result<()> {
    let scenario = reexamine_scenario()?;

    // d:p-rej is rejected, but d:open does not rest on it.
    let text = refusal(plan(&scenario, "d:open", Some("d:p-rej"))?);

    assert!(text.contains("does not rest on \"d:p-rej\""), "{text}");
    assert!(
        text.contains("it rests on \"Cap requests per tenant\" (d:p-sup)"),
        "{text}"
    );
    assert!(text.ends_with("nothing was written"), "{text}");
    Ok(())
}

#[test]
fn a_decision_that_rests_on_nothing_has_no_premise_to_have_changed() -> Result<()> {
    let scenario = reexamine_scenario()?;

    let text = refusal(plan(&scenario, "d:p-sup-new", None)?);

    assert!(
        text.contains("rests on no decision, assumption or bet"),
        "{text}"
    );
    Ok(())
}

#[test]
fn a_decision_that_does_not_exist_is_refused() -> Result<()> {
    let scenario = reexamine_scenario()?;

    let error = plan(&scenario, "d:nowhere", None).expect_err("an unknown decision is an error");

    assert!(
        error
            .to_string()
            .ends_with("decision does not exist: d:nowhere"),
        "{error}"
    );
    Ok(())
}

#[test]
fn an_empty_premise_is_refused_not_read_as_none() -> Result<()> {
    let scenario = reexamine_scenario()?;

    for premise in ["", "   "] {
        let text = refusal(plan(&scenario, "d:open", Some(premise))?);

        assert!(text.contains("premise to reaffirm is empty"), "{text}");
    }
    Ok(())
}

#[test]
fn the_check_writes_nothing() -> Result<()> {
    let scenario = reexamine_scenario()?;
    let before = scenario.ledger().read(0, 10_000)?.len();

    for (decision, premise) in [
        ("d:open", None),
        ("d:twice", None),
        ("d:after-sup", None),
        ("d:on-fine", Some("h:fine")),
    ] {
        let _ = plan(&scenario, decision, premise)?;
    }

    assert_eq!(scenario.ledger().read(0, 10_000)?.len(), before);
    Ok(())
}

#[test]
fn a_reaffirmation_closes_the_item_the_check_then_refuses() -> Result<()> {
    let scenario = two_open_items()?;
    let first = target(plan(&scenario, "d:leaf", Some("d:p1"))?);

    scenario.reaffirmed(
        &first.decision_id,
        &first.premise_id,
        "Sharding does not depend on the cap",
        "agent:codex:reviewer",
        "2026-09-04T00:00:00Z",
    )?;

    // d:p1 is closed, so d:p2 is now the only open item and needs no premise named.
    let second = target(plan(&scenario, "d:leaf", None)?);
    assert_eq!(second.premise_id, "d:p2");
    let again = refusal(plan(&scenario, "d:leaf", Some("d:p1"))?);
    assert!(again.contains("agent:codex:reviewer"), "{again}");
    assert!(
        again.contains("already reaffirmed after its latest change"),
        "{again}"
    );
    Ok(())
}
