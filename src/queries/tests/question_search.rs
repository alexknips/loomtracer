use crate::commands::{Commands, DecisionProposalInput, FloorInput, Grounding};
use crate::ledger::{AnyLedger, SqliteEventLedger};
use crate::projector::{memory::MemoryGraph, rebuild_graph};
use crate::Result;

use super::*;

// `search` asks again as `recall` does for a phrase of three or more words that no decision
// matches in full. The words are counted as the person wrote them: a question that is five words
// long with two content words ("where does the website live") is still a phrase
// (hivemind-vpf4q).

struct Fixture {
    _dir: tempfile::TempDir,
    ledger: AnyLedger,
    graph: MemoryGraph,
    website: String,
    interviews: String,
}

fn propose(commands: &Commands<'_, AnyLedger>, title: &str, rationale: &str) -> Result<String> {
    let option_id = commands.record_option("human:alex", "Do it", "Do it")?;
    commands.propose_decision(DecisionProposalInput {
        grounding: Grounding::NotAsked,
        expressed_confidence: None,
        actor_id: "human:alex",
        title,
        rationale,
        topic_keys: &["notes".to_owned()],
        option_ids: std::slice::from_ref(&option_id),
        option_labels: &["Do it".to_owned()],
        chosen_option_id: Some(option_id.as_str()),
        decided_by: None,
        still_proposed: false,
        hypothesis_ids: &[],
        evidence_ids: &[],
        quote: None,
        question: None,
        delegated_by: None,
        project: None,
        floor: FloorInput::NotAsked,
    })
}

fn fixture() -> Result<Fixture> {
    let dir = tempfile::tempdir().map_err(query_error)?;
    let ledger = AnyLedger::Sqlite(SqliteEventLedger::open(dir.path())?);
    let commands = Commands::new(&ledger);
    let website = propose(
        &commands,
        "The website lives in a public repo",
        "A public repo lets anyone read the pages",
    )?;
    let interviews = propose(
        &commands,
        "Interviews are framed around one decision at a time",
        "A single decision keeps the answers comparable",
    )?;
    propose(
        &commands,
        "Charge per seat",
        "Seats are the unit a team already counts",
    )?;
    let graph = MemoryGraph::default();
    rebuild_graph(&ledger, &graph)?;
    Ok(Fixture {
        _dir: dir,
        ledger,
        graph,
        website,
        interviews,
    })
}

fn ask(f: &Fixture, q: &str) -> Result<DecisionSearchResults> {
    let request = SearchDecisionRequest {
        query: Some(q.to_owned()),
        limit: 10,
        ..SearchDecisionRequest::default()
    };
    Ok(search_decisions_any(&QueryContext::local(), &f.ledger, &f.graph, &request)?.data)
}

fn ids(results: &DecisionSearchResults) -> Vec<&str> {
    results
        .items
        .iter()
        .map(|item| item.decision.id.as_str())
        .collect()
}

#[test]
fn a_question_with_two_content_words_is_a_phrase() -> Result<()> {
    let f = fixture()?;
    for (question, expected) in [
        ("where does the website live", &f.website),
        ("how should interviews be framed", &f.interviews),
    ] {
        let answer = ask(&f, question)?;
        assert_eq!(
            ids(&answer).first().copied(),
            Some(expected.as_str()),
            "{question:?} finds its decision first: {:?}",
            ids(&answer)
        );
        assert_eq!(
            answer.query.as_deref(),
            Some(question),
            "the query is echoed as it was given"
        );
    }
    Ok(())
}

#[test]
fn a_two_word_query_stays_exact() -> Result<()> {
    let f = fixture()?;
    // Every word, or nothing: a short query is not asked again.
    let answer = ask(&f, "website nothing")?;
    assert!(ids(&answer).is_empty(), "{:?}", ids(&answer));
    let answer = ask(&f, "website lives")?;
    assert_eq!(ids(&answer), [f.website.as_str()]);
    Ok(())
}

#[test]
fn a_phrase_of_only_question_words_lists_nothing() -> Result<()> {
    let f = fixture()?;
    // No word is left to ask with, and no words is not "every decision".
    let answer = ask(&f, "what did we decide")?;
    assert!(ids(&answer).is_empty(), "{:?}", ids(&answer));
    Ok(())
}
