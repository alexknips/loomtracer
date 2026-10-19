//! Chain-mode summaries of branched and re-asserted supersessions (hivemind-pydlc).

use super::*;

fn chain_summary<L: crate::ledger::EventLedger>(ledger: &L, anchor: &str) -> DecisionSummary {
    let request = SummarizeRequest {
        decision_ids: vec![anchor.to_owned()],
        mode: SummarizeMode::Chain,
    };
    summarize_decisions(&make_graph(ledger), &request)
        .unwrap()
        .data
}

fn seed_plain<L: crate::ledger::EventLedger>(commands: &Commands<'_, L>, title: &str) -> String {
    seed_decision(
        commands,
        "test-actor",
        title,
        "Because the chain-mode summary fixtures need a readable why",
        &["api"],
        &["approach"],
        None,
    )
}

#[test]
fn chain_mode_names_both_superseders_of_a_twice_superseded_decision() {
    let ledger = InMemoryEventLedger::new();
    let commands = Commands::new(&ledger);
    let old_id = seed_plain(&commands, "Original");
    let a_id = seed_plain(&commands, "Successor A");
    let b_id = seed_plain(&commands, "Successor B");
    commands
        .supersede_decision(&old_id, &a_id, "test-actor")
        .unwrap();
    commands
        .supersede_decision(&old_id, &b_id, "test-actor")
        .unwrap();

    let data = chain_summary(&ledger, &old_id);
    let summary = &data.summary;
    assert!(summary.contains("(1 steps)"), "{summary}");
    assert!(summary.contains("Branch: "), "{summary}");
    assert!(
        summary.contains("so no single decision is current"),
        "{summary}"
    );
    assert!(
        summary.contains("Successor A") && summary.contains("Successor B"),
        "{summary}"
    );
    assert!(summary.contains("Current status: none"), "{summary}");
    let mut expected = vec![old_id, a_id, b_id];
    expected.sort();
    let mut cited = data.cited_decision_ids.clone();
    cited.sort();
    assert_eq!(cited, expected, "every decision the summary names is cited");
    assert_eq!(data.cited_decision_ids.len(), 3);
}

#[test]
fn chain_mode_names_both_decisions_a_decision_superseded() {
    let ledger = InMemoryEventLedger::new();
    let commands = Commands::new(&ledger);
    let y_id = seed_plain(&commands, "Predecessor Y");
    let z_id = seed_plain(&commands, "Predecessor Z");
    let x_id = seed_plain(&commands, "Merged X");
    commands
        .supersede_decision(&y_id, &x_id, "test-actor")
        .unwrap();
    commands
        .supersede_decision(&z_id, &x_id, "test-actor")
        .unwrap();

    let data = chain_summary(&ledger, &x_id);
    let summary = &data.summary;
    assert!(summary.contains("supersedes 2 decisions"), "{summary}");
    assert!(
        summary.contains("Predecessor Y") && summary.contains("Predecessor Z"),
        "{summary}"
    );
    assert!(
        summary.contains("Current status: Proposed"),
        "the line ends at X, which stands: {summary}"
    );
    assert_eq!(data.cited_decision_ids.first(), Some(&x_id));
    assert_eq!(data.cited_decision_ids.len(), 3);
    assert!(data.cited_decision_ids.contains(&y_id) && data.cited_decision_ids.contains(&z_id));
}

#[test]
fn chain_mode_reads_a_pair_superseded_twice_as_one_step() {
    let ledger = InMemoryEventLedger::new();
    let commands = Commands::new(&ledger);
    let old_id = seed_plain(&commands, "Original");
    let new_id = seed_plain(&commands, "Replacement");
    commands
        .supersede_decision(&old_id, &new_id, "test-actor")
        .unwrap();
    // The same pair again, as a client retries after a timeout.
    commands
        .supersede_decision(&old_id, &new_id, "test-actor")
        .unwrap();

    let data = chain_summary(&ledger, &old_id);
    assert!(data.summary.contains("(2 steps)"), "{}", data.summary);
    assert!(!data.summary.contains("Branch:"), "{}", data.summary);
    assert_eq!(data.cited_decision_ids, vec![old_id, new_id]);
}
