//! Take it upstream (hivemind-ottw.5): the `disagree` and `supersede` replies list what the
//! decision they act on rests on and which of that still stands, and the plugins' prose asks
//! whether the disagreement is with the decision or with a premise before it is recorded.
//!
//! The hint changes nothing: the leaf action is recorded exactly as before, and nothing is
//! recorded for a premise.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use clap::Parser;
use hivemind::cli::{run, Cli};
use hivemind::events::{Event, EventType};
use hivemind::ledger::{EventLedger, SqliteEventLedger};
use serde_json::{json, Value};
use uuid::Uuid;

type TestResult<T> = std::result::Result<T, Box<dyn std::error::Error>>;

const CAPTURER: &str = "human:alice";
const GROUNDER: &str = "agent:claude:grounder";
const DISSENTER: &str = "human:bob";

struct Scratch(PathBuf);

impl Scratch {
    fn new(label: &str) -> TestResult<Self> {
        let path =
            std::env::temp_dir().join(format!("hivemind-upstream-{label}-{}", Uuid::new_v4()));
        fs::create_dir_all(&path)?;
        Ok(Self(path))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn run_cli(dir: &Path, actor: &str, json: bool, args: &[&str]) -> hivemind::Result<String> {
    let mut argv = vec![
        "hivemind".to_owned(),
        "--hivemind-dir".to_owned(),
        dir.display().to_string(),
        "--actor".to_owned(),
        actor.to_owned(),
    ];
    if json {
        argv.push("--json".to_owned());
    }
    argv.extend(args.iter().map(|arg| (*arg).to_owned()));
    run(&Cli::parse_from(argv))
}

/// Capture a decision the raw way, which self-accepts it for the capturer.
fn capture_raw(dir: &Path, title: &str) -> TestResult<String> {
    let output = run_cli(
        dir,
        CAPTURER,
        true,
        &[
            "emit",
            "decision.proposed",
            "--title",
            title,
            "--rationale",
            "Rationale text long enough for the readable floor",
            "--topic-keys",
            "upstream",
            "--options",
            "adopt,skip",
            "--chose",
            "adopt",
        ],
    )?;
    let envelope: Value = serde_json::from_str(&output)?;
    Ok(envelope["value"]
        .as_str()
        .ok_or("emit should return the decision id")?
        .to_owned())
}

/// Like [`capture_raw`], but nobody has decided it: it stays `proposed`.
fn capture_proposed(dir: &Path, title: &str) -> TestResult<String> {
    let output = run_cli(
        dir,
        CAPTURER,
        true,
        &[
            "emit",
            "decision.proposed",
            "--title",
            title,
            "--rationale",
            "Rationale text long enough for the readable floor",
            "--topic-keys",
            "upstream",
            "--options",
            "adopt,skip",
        ],
    )?;
    let envelope: Value = serde_json::from_str(&output)?;
    Ok(envelope["value"]
        .as_str()
        .ok_or("emit should return the decision id")?
        .to_owned())
}

/// `dependent` rests on `premise`, attributed later.
fn rest_on(dir: &Path, dependent: &str, premise: &str) -> TestResult<()> {
    run_cli(
        dir,
        GROUNDER,
        true,
        &["ground", dependent, "--rests-on-decision", premise],
    )?;
    Ok(())
}

const PREMISE: &str = "Cap requests per tenant";
const LEAF: &str = "Shard by tenant";
const SIBLING: &str = "Price by seat";

/// "Shard by tenant" and "Price by seat" both rest on "Cap requests per tenant", which nobody
/// has questioned. Returns the premise id.
fn two_decisions_on_one_premise(dir: &Path) -> TestResult<String> {
    let premise = capture_raw(dir, PREMISE)?;
    capture_raw(dir, LEAF)?;
    capture_raw(dir, SIBLING)?;
    rest_on(dir, LEAF, PREMISE)?;
    rest_on(dir, SIBLING, PREMISE)?;
    Ok(premise)
}

fn events(dir: &Path) -> TestResult<Vec<Event>> {
    Ok(SqliteEventLedger::open(dir)?.read(0, 1000)?)
}

fn supersede_args<'a>(old: &'a str, title: &'a str) -> Vec<&'a str> {
    vec![
        "supersede",
        old,
        "--title",
        title,
        "--rationale",
        "The replacement keeps the premise and changes how the work is cut",
        "--topic-keys",
        "upstream",
        "--options",
        "Keep it,Replace it",
        "--chose",
        "Replace it",
        "--rests-on-decision",
        PREMISE,
        "--binds-nothing",
        "--cheap-to-undo",
    ]
}

#[test]
fn disagree_lists_what_the_decision_rests_on_and_the_premise_that_still_holds() -> TestResult<()> {
    let scratch = Scratch::new("disagree")?;
    let dir = scratch.path();
    let premise = two_decisions_on_one_premise(dir)?;
    let before = events(dir)?.len();

    let reply: Value = serde_json::from_str(&run_cli(
        dir,
        DISSENTER,
        true,
        &[
            "disagree",
            LEAF,
            "--reason",
            "Tenants differ too much in size",
        ],
    )?)?;

    assert_eq!(
        reply["raise_upstream"],
        json!([{
            "premise_id": premise,
            "label": PREMISE,
            "other_dependents": 1,
        }])
    );
    let rests_on = reply["rests_on"].as_array().ok_or("rests_on is a list")?;
    assert_eq!(rests_on.len(), 1, "{reply}");
    assert_eq!(rests_on[0]["kind"], "decision");
    assert_eq!(rests_on[0]["id"], premise.as_str());
    assert_eq!(rests_on[0]["label"], PREMISE);
    assert_eq!(rests_on[0]["state"], "holds");
    assert_eq!(rests_on[0]["dependents_count"], 2);
    assert_eq!(reply["decision_status"], "contested");

    // The disagreement is the one event it always was: nothing for the premise, no
    // reaffirmation, no side effect.
    let appended: Vec<Event> = events(dir)?.into_iter().skip(before).collect();
    assert_eq!(appended.len(), 1, "{appended:?}");
    assert_eq!(appended[0].event_type, EventType::DecisionRejected);
    assert_eq!(appended[0].actor_id, DISSENTER);
    assert_eq!(appended[0].payload["decision_id"], reply["decision_id"]);
    Ok(())
}

#[test]
fn disagree_text_adds_one_line_per_premise_and_leaves_the_first_line_alone() -> TestResult<()> {
    let scratch = Scratch::new("disagree-text")?;
    let dir = scratch.path();
    let premise = two_decisions_on_one_premise(dir)?;

    let text = run_cli(
        dir,
        DISSENTER,
        false,
        &[
            "disagree",
            LEAF,
            "--reason",
            "Tenants differ too much in size",
        ],
    )?;

    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 2, "{text}");
    assert!(
        lines[0].starts_with("event_id=") && lines[0].ends_with("status=contested"),
        "{text}"
    );
    assert!(
        lines[1].starts_with(&format!(
            "raise_upstream: {premise} \"{PREMISE}\" (1 other decision rests on it;"
        )),
        "{text}"
    );

    // Singular and plural read plainly: with the first one also disagreed with, the sibling's
    // premise still has one other decision resting on it; a lone dependent has none.
    let lone = Scratch::new("disagree-lone")?;
    let lone_premise = capture_raw(lone.path(), PREMISE)?;
    capture_raw(lone.path(), LEAF)?;
    rest_on(lone.path(), LEAF, PREMISE)?;
    let text = run_cli(
        lone.path(),
        DISSENTER,
        false,
        &[
            "disagree",
            LEAF,
            "--reason",
            "Tenants differ too much in size",
        ],
    )?;
    assert!(
        text.contains(&format!(
            "raise_upstream: {lone_premise} \"{PREMISE}\" (no other decision rests on it;"
        )),
        "{text}"
    );
    Ok(())
}

#[test]
fn a_decision_that_rests_on_nothing_has_nothing_to_raise() -> TestResult<()> {
    let scratch = Scratch::new("nothing")?;
    let dir = scratch.path();
    two_decisions_on_one_premise(dir)?;

    let reply: Value = serde_json::from_str(&run_cli(
        dir,
        DISSENTER,
        true,
        &["disagree", PREMISE, "--reason", "Caps hide real load"],
    )?)?;
    assert_eq!(reply["raise_upstream"], json!([]), "{reply}");
    assert_eq!(reply["rests_on"], json!([]), "{reply}");

    let text = run_cli(
        dir,
        DISSENTER,
        false,
        &["disagree", SIBLING, "--reason", "Seats are the wrong unit"],
    )?;
    // The premise was just contested: it is no longer a premise that "still holds", so the
    // disagreement is already on record and there is nothing to raise.
    assert_eq!(text.lines().count(), 1, "{text}");
    Ok(())
}

#[test]
fn a_premise_that_no_longer_stands_is_shown_but_not_offered() -> TestResult<()> {
    let scratch = Scratch::new("stale")?;
    let dir = scratch.path();
    // The premise was only ever proposed, and someone rejects it: nobody accepted it, so it
    // reads rejected, not contested.
    let premise = capture_proposed(dir, PREMISE)?;
    capture_raw(dir, LEAF)?;
    rest_on(dir, LEAF, PREMISE)?;
    run_cli(
        dir,
        DISSENTER,
        true,
        &["disagree", PREMISE, "--reason", "Caps hide real load"],
    )?;

    let reply: Value = serde_json::from_str(&run_cli(
        dir,
        DISSENTER,
        true,
        &[
            "disagree",
            LEAF,
            "--reason",
            "Tenants differ too much in size",
        ],
    )?)?;

    assert_eq!(reply["raise_upstream"], json!([]), "{reply}");
    assert_eq!(reply["rests_on"][0]["id"], premise.as_str());
    assert_eq!(reply["rests_on"][0]["state"], "rejected", "{reply}");
    Ok(())
}

#[test]
fn supersede_lists_the_premises_of_the_replaced_decision_without_counting_the_replacement(
) -> TestResult<()> {
    let scratch = Scratch::new("supersede")?;
    let dir = scratch.path();
    let premise = two_decisions_on_one_premise(dir)?;
    let before = events(dir)?.len();

    let reply: Value = serde_json::from_str(&run_cli(
        dir,
        DISSENTER,
        true,
        &supersede_args(LEAF, "Shard by region"),
    )?)?;

    // Of the decisions resting on the premise, the sibling is the only "other": the replaced
    // decision and its replacement, which rests on the same premise, are the same act.
    assert_eq!(
        reply["raise_upstream"],
        json!([{
            "premise_id": premise,
            "label": PREMISE,
            "other_dependents": 1,
        }])
    );
    // `rests_on` stays what the replacement rests on.
    assert_eq!(reply["rests_on"][0]["id"], premise.as_str(), "{reply}");
    assert_eq!(reply["old_decision_status"], "superseded");

    let appended: Vec<Event> = events(dir)?.into_iter().skip(before).collect();
    assert!(
        appended
            .iter()
            .any(|event| event.event_type == EventType::DecisionSuperseded),
        "{appended:?}"
    );
    assert!(
        appended
            .iter()
            .all(|event| event.event_type != EventType::DecisionReaffirmed),
        "a supersession never closes a re-examine item: {appended:?}"
    );

    // Text reads the same, with the replacement counted out again for the next one.
    let text = run_cli(
        dir,
        DISSENTER,
        false,
        &supersede_args(SIBLING, "Price by usage"),
    )?;
    assert!(
        text.contains(&format!(
            "\nraise_upstream: {premise} \"{PREMISE}\" (2 other decisions rest on it;"
        )),
        "{text}"
    );
    assert!(text.starts_with("proposal_event_id="), "{text}");
    Ok(())
}

#[test]
fn the_disagree_and_supersede_scripts_pass_the_hint_through() -> TestResult<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let scratch = Scratch::new("scripts")?;
    let dir = scratch.path();
    let premise = two_decisions_on_one_premise(dir)?;

    let run_script = |script: &str, args: &[&str]| -> TestResult<std::process::Output> {
        Ok(
            Command::new(root.join(format!("plugins/hivemind-context/scripts/{script}.sh")))
                .current_dir(root)
                .env("HIVEMIND_CAPTURE_BIN", env!("CARGO_BIN_EXE_hivemind"))
                .env("HIVEMIND_DIR", dir)
                .env("CLAUDE_PROJECT_DIR", root)
                .env("CLAUDE_SESSION_ID", "plugin-upstream-session")
                .env_remove("GC_AGENT")
                .env_remove("GC_ALIAS")
                .args(args)
                .output()?,
        )
    };

    for script in ["disagree", "supersede"] {
        let help = run_script(script, &["--help"])?;
        let usage = String::from_utf8_lossy(&help.stderr);
        assert!(help.status.success(), "{script} --help");
        assert!(
            usage.contains("raise_upstream") && usage.contains("verify.sh"),
            "{script} usage names the hint and the check: {usage}"
        );
    }

    let disagreed = run_script(
        "disagree",
        &[
            "Shard",
            "by",
            "tenant",
            "--reason",
            "Tenants differ too much in size",
        ],
    )?;
    assert!(
        disagreed.status.success(),
        "disagree.sh failed: {}",
        String::from_utf8_lossy(&disagreed.stderr)
    );
    let stdout = String::from_utf8_lossy(&disagreed.stdout);
    assert!(
        stdout.contains(&format!("\nraise_upstream: {premise} \"{PREMISE}\"")),
        "{stdout}"
    );
    Ok(())
}

/// The prose is the agent-side half of the feature: each place that tells an agent how to
/// disagree or replace a decision asks the question first. These pin that it stays asked.
#[test]
fn every_place_that_teaches_disagree_and_supersede_asks_the_question_first() -> TestResult<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let question_in_prose = [
        (
            "plugins/hivemind-context/skills/hivemind-context/SKILL.md",
            vec![
                "Take it upstream",
                "verify",
                "with this decision or with the premise it rests on",
                "raise_upstream",
                "never dropped",
            ],
        ),
        (
            "plugins/hivemind-context/commands/disagree.md",
            vec![
                "Take it upstream",
                "scripts/verify.sh",
                "with this decision or with a",
                "raise_upstream:",
                "never dropped",
            ],
        ),
        (
            "plugins/hivemind-context/commands/supersede.md",
            vec![
                "Take it upstream",
                "scripts/verify.sh",
                "with this",
                "raise_upstream:",
                "never dropped",
            ],
        ),
        (
            "plugins/hivemind-context/skills/hivemind-context/agents/openai.yaml",
            vec![
                "run verify",
                "whether the disagreement is with the decision or with a premise",
                "never drop the action",
            ],
        ),
        (
            "plugins/hivemind-context/.codex-plugin/plugin.json",
            vec!["whether the disagreement is with the decision or with a premise"],
        ),
        (
            "plugins/hivemind-capture/skills/hivemind-capture/SKILL.md",
            vec![
                "`verify` on it first",
                "with the decision or with the premise",
                "raise_upstream",
            ],
        ),
    ];
    for (path, needles) in question_in_prose {
        let text = fs::read_to_string(root.join(path))?;
        for needle in needles {
            // Prose wraps; compare with the line breaks folded to single spaces.
            let folded = text.split_whitespace().collect::<Vec<_>>().join(" ");
            let needle = needle.split_whitespace().collect::<Vec<_>>().join(" ");
            assert!(folded.contains(&needle), "{path} should say `{needle}`");
        }
    }

    // The commands can run the check they now ask for.
    for command in ["disagree", "supersede"] {
        let text = fs::read_to_string(
            root.join(format!("plugins/hivemind-context/commands/{command}.md")),
        )?;
        let allowed = text
            .lines()
            .find(|line| line.starts_with("allowed-tools:"))
            .ok_or("allowed-tools frontmatter")?;
        assert!(
            allowed.contains("scripts/verify.sh"),
            "{command}: {allowed}"
        );
    }
    Ok(())
}
