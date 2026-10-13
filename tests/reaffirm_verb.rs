//! `hivemind reaffirm` and `hivemind reexamine` (hivemind-ottw.4): say that a decision still
//! holds after something it rests on changed, and list what is waiting for that. CLI behaviour,
//! the refusals that must write nothing, and the hivemind-context plugin scripts that wrap the
//! verbs.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use clap::Parser;
use hivemind::cli::{run, Cli};
use hivemind::events::{Event, EventType};
use hivemind::ledger::{EventLedger, SqliteEventLedger};
use serde_json::Value;
use uuid::Uuid;

type TestResult<T> = std::result::Result<T, Box<dyn std::error::Error>>;

const CAPTURER: &str = "human:alice";
const GROUNDER: &str = "agent:claude:grounder";
const DISSENTER: &str = "human:bob";
const REVIEWER: &str = "agent:codex:reviewer";
const REASON: &str = "Sharding by tenant never depended on the per-tenant cap";

struct Scratch(PathBuf);

impl Scratch {
    fn new(label: &str) -> TestResult<Self> {
        let path =
            std::env::temp_dir().join(format!("hivemind-reaffirm-{label}-{}", Uuid::new_v4()));
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
            "reaffirm",
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

/// Someone else disagrees, so the decision both accepted and rejected is contested.
fn contest(dir: &Path, title: &str) -> TestResult<()> {
    run_cli(
        dir,
        DISSENTER,
        true,
        &[
            "disagree",
            title,
            "--reason",
            "Per-user caps fit billing better",
        ],
    )?;
    Ok(())
}

/// "Shard by tenant" rests on "Cap requests per tenant", which `DISSENTER` contests: one open
/// item. Returns `(premise_id, dependent_id)`.
fn contested_premise_with_a_dependent(dir: &Path) -> TestResult<(String, String)> {
    let premise = capture_raw(dir, "Cap requests per tenant")?;
    let dependent = capture_raw(dir, "Shard by tenant")?;
    rest_on(dir, "Shard by tenant", "Cap requests per tenant")?;
    contest(dir, "Cap requests per tenant")?;
    Ok((premise, dependent))
}

fn reaffirm(dir: &Path, args: &[&str]) -> hivemind::Result<String> {
    let mut full = vec!["reaffirm"];
    full.extend_from_slice(args);
    run_cli(dir, REVIEWER, true, &full)
}

fn reexamine(dir: &Path, args: &[&str]) -> TestResult<Value> {
    let mut full = vec!["reexamine"];
    full.extend_from_slice(args);
    Ok(serde_json::from_str(&run_cli(dir, REVIEWER, true, &full)?)?)
}

fn events(dir: &Path) -> TestResult<Vec<Event>> {
    Ok(SqliteEventLedger::open(dir)?.read(0, 1000)?)
}

fn reaffirmed_events(dir: &Path) -> TestResult<Vec<Event>> {
    Ok(events(dir)?
        .into_iter()
        .filter(|event| event.event_type == EventType::DecisionReaffirmed)
        .collect())
}

fn items(listing: &Value) -> TestResult<&Vec<Value>> {
    Ok(listing["data"]["items"]
        .as_array()
        .ok_or("the listing has items")?)
}

fn assert_refused_writing_nothing(
    dir: &Path,
    args: &[&str],
    expected: &str,
) -> TestResult<hivemind::HivemindError> {
    let before = events(dir)?.len();
    let error = reaffirm(dir, args).expect_err("the call must be refused");
    assert!(
        error.to_string().contains(expected),
        "expected `{expected}` in: {error}"
    );
    assert_eq!(
        events(dir)?.len(),
        before,
        "a refused reaffirm call writes nothing"
    );
    Ok(error)
}

#[test]
fn reaffirm_closes_the_open_item_and_attributes_it_to_the_reaffirmer() -> TestResult<()> {
    let scratch = Scratch::new("end-to-end")?;
    let dir = scratch.path();
    let (premise_id, dependent_id) = contested_premise_with_a_dependent(dir)?;
    let before = events(dir)?.len();

    let open = reexamine(dir, &[])?;
    let listed = items(&open)?;
    assert_eq!(listed.len(), 1, "{open}");
    assert_eq!(listed[0]["dependent"]["id"], dependent_id.as_str());
    assert_eq!(listed[0]["dependent"]["title"], "Shard by tenant");
    assert_eq!(listed[0]["premise_id"], premise_id.as_str());
    assert_eq!(listed[0]["premise_label"], "Cap requests per tenant");
    assert_eq!(listed[0]["change"]["kind"], "contested");
    assert_eq!(listed[0]["state"], "open");

    let reply: Value =
        serde_json::from_str(&reaffirm(dir, &["Shard by tenant", "--because", REASON])?)?;
    assert_eq!(reply["decision_id"], dependent_id.as_str());
    assert_eq!(reply["decision_title"], "Shard by tenant");
    assert_eq!(reply["premise_id"], premise_id.as_str());
    assert_eq!(reply["premise_kind"], "decision");
    assert_eq!(reply["actor_id"], REVIEWER);
    assert_eq!(reply["reason"], REASON);
    assert_eq!(reply["covers"][0]["kind"], "contested");
    assert!(reply["event_id"].as_u64().is_some(), "{reply}");

    // Exactly one event, by the reaffirmer, naming the dependent and the premise.
    let appended: Vec<Event> = events(dir)?.into_iter().skip(before).collect();
    assert_eq!(appended.len(), 1, "no fan-out: {appended:?}");
    assert_eq!(appended[0].event_type, EventType::DecisionReaffirmed);
    assert_eq!(appended[0].actor_id, REVIEWER);
    assert_eq!(appended[0].payload["decision_id"], dependent_id.as_str());
    assert_eq!(appended[0].payload["premise_id"], premise_id.as_str());
    assert_eq!(appended[0].payload["reason"], REASON);
    assert_eq!(reply["event_id"], serde_json::json!(appended[0].event_id));

    // The item is closed now: nothing open, and `--all` shows who closed it and why.
    assert!(items(&reexamine(dir, &[])?)?.is_empty());
    let all = reexamine(dir, &["--all"])?;
    let closed = items(&all)?;
    assert_eq!(closed.len(), 1);
    assert_eq!(closed[0]["state"], "reaffirmed");
    assert_eq!(closed[0]["reaffirmations"][0]["actor_id"], REVIEWER);
    assert_eq!(closed[0]["reaffirmations"][0]["reason"], REASON);

    // A contested premise is not a verdict: reaffirming it never flipped `held_up`.
    let verified: Value = serde_json::from_str(&run_cli(
        dir,
        CAPTURER,
        true,
        &["query", "verify", "Shard by tenant"],
    )?)?;
    assert_eq!(
        verified["data"]["still_holds"]["held_up"], true,
        "{verified}"
    );
    Ok(())
}

#[test]
fn reaffirm_and_reexamine_read_plainly_in_text() -> TestResult<()> {
    let scratch = Scratch::new("text")?;
    let dir = scratch.path();
    let (premise_id, dependent_id) = contested_premise_with_a_dependent(dir)?;

    let listing = run_cli(dir, REVIEWER, false, &["reexamine"])?;
    let mut lines = listing.lines();
    assert_eq!(lines.next(), Some("1 open re-examine item(s)"), "{listing}");
    let item = lines.next().ok_or("an item line")?;
    assert!(
        item.starts_with(&format!("  OPEN \"Shard by tenant\" ({dependent_id})")),
        "{item}"
    );
    assert!(
        item.contains(&format!(
            "on \"Cap requests per tenant\" ({premise_id}) contested"
        )),
        "{item}"
    );

    let text = run_cli(
        dir,
        REVIEWER,
        false,
        &["reaffirm", "Shard by tenant", "--because", REASON],
    )?;
    assert!(
        text.starts_with(&format!(
            "reaffirmed \"Shard by tenant\" ({dependent_id}): still holds after \"Cap requests per tenant\" ({premise_id}) was contested"
        )),
        "{text}"
    );
    assert!(
        text.contains(&format!("by {REVIEWER}: \"{REASON}\"")),
        "{text}"
    );

    assert_eq!(
        run_cli(dir, REVIEWER, false, &["reexamine"])?,
        "no open re-examine items"
    );
    let all = run_cli(dir, REVIEWER, false, &["reexamine", "--all"])?;
    assert!(all.starts_with("1 re-examine item(s)"), "{all}");
    assert!(
        all.contains(&format!("REAFFIRMED \"Shard by tenant\" ({dependent_id})")),
        "{all}"
    );
    // Who, when (the date is the run's own, so it is not pinned) and why.
    assert!(
        all.contains(&format!("reaffirmed by {REVIEWER} on "))
            && all.contains(&format!(": \"{REASON}\"")),
        "{all}"
    );
    Ok(())
}

#[test]
fn reaffirm_refuses_what_there_is_nothing_to_reexamine_and_writes_nothing() -> TestResult<()> {
    let scratch = Scratch::new("refusals")?;
    let dir = scratch.path();
    let (_premise_id, _dependent_id) = contested_premise_with_a_dependent(dir)?;
    // A decision that rests on a premise nobody has touched.
    capture_raw(dir, "Keep audit boundaries")?;
    capture_raw(dir, "Write audit logs")?;
    rest_on(dir, "Write audit logs", "Keep audit boundaries")?;

    // Nothing has changed under it.
    assert_refused_writing_nothing(
        dir,
        &["Write audit logs", "--because", "Checked"],
        "none of the 1 premise(s)",
    )?;
    assert_refused_writing_nothing(
        dir,
        &[
            "Write audit logs",
            "--premise",
            "audit boundaries",
            "--because",
            "Checked",
        ],
        "has not changed",
    )?;
    // The decision does not rest on the premise it was told about.
    assert_refused_writing_nothing(
        dir,
        &[
            "Write audit logs",
            "--premise",
            "Cap requests per tenant",
            "--because",
            "Checked",
        ],
        "does not rest on \"Cap requests per tenant\"",
    )?;
    // A reason is part of the statement.
    assert_refused_writing_nothing(
        dir,
        &["Shard by tenant", "--because", "   "],
        "reason must not be empty",
    )?;

    // The first call records; the same call again is refused and names the first, so a retry
    // sees what happened.
    reaffirm(dir, &["Shard by tenant", "--because", REASON])?;
    let after_first = events(dir)?.len();
    let error = assert_refused_writing_nothing(
        dir,
        &["Shard by tenant", "--because", "Saying it again"],
        "already reaffirmed after its latest change",
    )?;
    let message = error.to_string();
    let first = &reaffirmed_events(dir)?[0];
    assert!(message.contains(REVIEWER), "who: {message}");
    assert!(
        message.contains(&format!(
            "(event {}, ",
            first.event_id.ok_or("a ledger number")?
        )),
        "the ledger number the first reply gave: {message}"
    );
    assert!(
        message.contains(&format!("id {}", first.event_uuid)),
        "its uuid: {message}"
    );
    assert!(message.contains(REASON), "{message}");
    assert!(message.ends_with("nothing was written"), "{message}");
    assert_eq!(events(dir)?.len(), after_first);

    // Once the premise changes again (it is superseded), the item is open again and the
    // reaffirmation is needed once more; the earlier one stays on record.
    run_cli(
        dir,
        CAPTURER,
        true,
        &[
            "supersede",
            "Cap requests per tenant",
            "--title",
            "Cap requests per user",
            "--rationale",
            "Billing is per user, so the cap should be too, whatever the tenant size",
            "--topic-keys",
            "reaffirm",
            "--options",
            "Per user,Per tenant",
            "--chose",
            "Per user",
            "--rests-on-assumption",
            "Billing stays per user",
            "--binds-nothing",
            "--cheap-to-undo",
        ],
    )?;
    let reopened = reexamine(dir, &[])?;
    let listed = items(&reopened)?;
    assert_eq!(listed.len(), 1, "{reopened}");
    assert_eq!(listed[0]["change"]["kind"], "superseded");
    assert_eq!(
        listed[0]["earlier_reaffirmations"][0]["actor_id"], REVIEWER,
        "the earlier reaffirmation stays visible"
    );
    reaffirm(
        dir,
        &[
            "Shard by tenant",
            "--because",
            "Checked against the per-user cap too",
        ],
    )?;
    assert_eq!(reaffirmed_events(dir)?.len(), 2);
    Ok(())
}

#[test]
fn reaffirm_on_the_decision_that_solely_superseded_its_premise_says_so_and_writes_nothing(
) -> TestResult<()> {
    let scratch = Scratch::new("revision")?;
    let dir = scratch.path();
    let premise = capture_raw(dir, "Cap requests per tenant")?;
    let revision = capture_raw(dir, "Throttle each account by plan tier")?;
    rest_on(
        dir,
        "Throttle each account by plan tier",
        "Cap requests per tenant",
    )?;
    run_cli(
        dir,
        CAPTURER,
        true,
        &[
            "emit",
            "decision.superseded",
            "--old",
            &premise,
            "--new",
            &revision,
        ],
    )?;

    // The premise changed, and this decision is what changed it: that is the reason, never "has
    // not changed". By its words, by its id, and with no premise named at all.
    for premise_args in [
        &["--premise", "per tenant"][..],
        &["--premise", premise.as_str()][..],
        &[][..],
    ] {
        let mut args = vec![revision.as_str(), "--because", "Checked"];
        args.extend_from_slice(premise_args);
        let error = assert_refused_writing_nothing(
            dir,
            &args,
            "is the decision that superseded \"Cap requests per tenant\"",
        )?;
        let message = error.to_string();
        assert!(!message.contains("has not changed"), "{message}");
        assert!(!message.contains("has changed"), "{message}");
        assert!(message.ends_with("nothing was written"), "{message}");
    }
    Ok(())
}

#[test]
fn reaffirm_lists_several_open_items_and_the_premise_flag_names_one() -> TestResult<()> {
    let scratch = Scratch::new("several")?;
    let dir = scratch.path();
    let (first_premise, _dependent_id) = contested_premise_with_a_dependent(dir)?;
    let second_premise = capture_raw(dir, "Bill monthly")?;
    rest_on(dir, "Shard by tenant", "Bill monthly")?;
    contest(dir, "Bill monthly")?;
    let before = events(dir)?.len();

    // Two open items and no premise named: listed, nothing written, exit 0.
    let listing: Value =
        serde_json::from_str(&reaffirm(dir, &["Shard by tenant", "--because", REASON])?)?;
    assert_eq!(listing["data"]["outcome"], "ambiguous");
    assert_eq!(listing["data"]["field"], "premise");
    let premises: Vec<&str> = listing["data"]["items"]
        .as_array()
        .ok_or("items")?
        .iter()
        .filter_map(|item| item["premise_id"].as_str())
        .collect();
    assert_eq!(premises.len(), 2, "{listing}");
    assert!(premises.contains(&first_premise.as_str()));
    assert!(premises.contains(&second_premise.as_str()));
    assert_eq!(
        events(dir)?.len(),
        before,
        "listing the items wrote nothing"
    );

    let text = run_cli(
        dir,
        REVIEWER,
        false,
        &["reaffirm", "Shard by tenant", "--because", REASON],
    )?;
    assert!(
        text.contains("has 2 open re-examine items, and nothing was written"),
        "{text}"
    );
    assert!(text.contains("--premise"), "{text}");

    // Naming one (by words from its title, or by its id) closes that one only.
    let named: Value = serde_json::from_str(&reaffirm(
        dir,
        &["Shard by tenant", "--premise", "bill", "--because", REASON],
    )?)?;
    assert_eq!(named["premise_id"], second_premise.as_str());
    assert_eq!(reaffirmed_events(dir)?.len(), 1);
    let remaining = reexamine(dir, &[])?;
    assert_eq!(items(&remaining)?.len(), 1);

    // One item is left, so no premise is needed any more.
    let last: Value = serde_json::from_str(&reaffirm(
        dir,
        &[
            "Shard by tenant",
            "--because",
            "The cap does not touch sharding either",
        ],
    )?)?;
    assert_eq!(last["premise_id"], first_premise.as_str());
    assert!(items(&reexamine(dir, &[])?)?.is_empty());
    Ok(())
}

#[test]
fn an_ambiguous_or_unmatched_description_is_listed_and_writes_nothing() -> TestResult<()> {
    let scratch = Scratch::new("ambiguous")?;
    let dir = scratch.path();
    capture_raw(dir, "Adopt the queue")?;
    capture_raw(dir, "Adopt the queue")?;
    let before = events(dir)?.len();

    let ambiguous = reaffirm(dir, &["Adopt the queue", "--because", REASON])?;
    assert!(ambiguous.contains("ambiguous"), "{ambiguous}");
    let unmatched = reaffirm(dir, &["Zebra crossing", "--because", REASON])?;
    assert!(unmatched.contains("not_found"), "{unmatched}");
    assert_eq!(events(dir)?.len(), before);

    // `#N` continues from the last listing, `--pick N` chooses; neither reaches the open-item
    // rule before a decision is picked, and the picked decision has nothing to re-examine.
    assert_refused_writing_nothing(
        dir,
        &["Adopt the queue", "--pick", "1", "--because", REASON],
        "rests on no decision, assumption or bet",
    )?;
    Ok(())
}

#[test]
fn reexamine_narrows_pages_and_says_when_it_stopped() -> TestResult<()> {
    let scratch = Scratch::new("listing")?;
    let dir = scratch.path();
    contested_premise_with_a_dependent(dir)?;
    capture_raw(dir, "Index requests by tenant")?;
    rest_on(dir, "Index requests by tenant", "Cap requests per tenant")?;

    let both = reexamine(dir, &[])?;
    assert_eq!(items(&both)?.len(), 2, "{both}");
    assert_eq!(both["truncated"], false);

    // A page of one says it stopped, and the cursor continues without repeating or skipping.
    let first_page = reexamine(dir, &["--limit", "1"])?;
    assert_eq!(items(&first_page)?.len(), 1);
    assert_eq!(first_page["truncated"], true, "{first_page}");
    let cursor = first_page["data"]["next_cursor"]
        .as_str()
        .ok_or("a truncated page says where to continue")?
        .to_owned();
    let second_page = reexamine(dir, &["--limit", "1", "--cursor", &cursor])?;
    assert_eq!(items(&second_page)?.len(), 1);
    assert_eq!(second_page["truncated"], false);
    assert_ne!(
        items(&first_page)?[0]["dependent"]["id"],
        items(&second_page)?[0]["dependent"]["id"]
    );
    let text = run_cli(dir, REVIEWER, false, &["reexamine", "--limit", "1"])?;
    assert!(
        text.contains("truncated=true next_cursor="),
        "text says it stopped: {text}"
    );

    // Narrowing to one decision resolves it like every fluent verb.
    let narrowed = reexamine(dir, &["--decision", "Index requests by tenant"])?;
    assert_eq!(items(&narrowed)?.len(), 1);
    assert_eq!(
        items(&narrowed)?[0]["dependent"]["title"],
        "Index requests by tenant"
    );
    let unmatched = reexamine(dir, &["--decision", "Zebra crossing"])?;
    assert_eq!(unmatched["data"]["outcome"], "not_found");

    // A project nobody registered is refused, not answered empty.
    let error = run_cli(dir, REVIEWER, true, &["reexamine", "--project", "nowhere"])
        .expect_err("an unregistered project is refused");
    assert!(error.to_string().contains("not registered"), "{error}");
    Ok(())
}

#[test]
fn reaffirm_and_reexamine_scripts_wrap_the_verbs() -> TestResult<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let scratch = Scratch::new("scripts")?;
    let dir = scratch.path();
    let (premise_id, dependent_id) = contested_premise_with_a_dependent(dir)?;
    let already = events(dir)?.len();

    let run_script = |script: &str, args: &[&str]| -> TestResult<std::process::Output> {
        Ok(
            Command::new(root.join(format!("plugins/hivemind-context/scripts/{script}.sh")))
                .current_dir(root)
                .env("HIVEMIND_CAPTURE_BIN", env!("CARGO_BIN_EXE_hivemind"))
                .env("HIVEMIND_DIR", dir)
                .env("CLAUDE_PROJECT_DIR", root)
                .env("CLAUDE_SESSION_ID", "plugin-reaffirm-session")
                .env_remove("GC_AGENT")
                .env_remove("GC_ALIAS")
                .args(args)
                .output()?,
        )
    };

    for script in ["reaffirm", "reexamine"] {
        let help = run_script(script, &["--help"])?;
        assert!(help.status.success(), "{script} --help");
        assert!(
            String::from_utf8_lossy(&help.stderr).contains(&format!("{script}.sh")),
            "{script} names itself in its usage"
        );
    }

    // `reexamine.sh` with no argument at all lists everything (and survives macOS bash 3.2).
    let listed = run_script("reexamine", &[])?;
    assert!(
        listed.status.success(),
        "reexamine.sh failed: {}",
        String::from_utf8_lossy(&listed.stderr)
    );
    let listing = String::from_utf8_lossy(&listed.stdout);
    assert!(
        listing.starts_with("1 open re-examine item(s)"),
        "{listing}"
    );
    assert!(listing.contains(&dependent_id), "{listing}");

    // A description typed as bare words, no quotes: the plugin joins it into one argument. The
    // reaffirmation is attributed to the agent the script derives, not the git-derived human.
    let reaffirmed = run_script("reaffirm", &["Shard", "by", "tenant", "--because", REASON])?;
    assert!(
        reaffirmed.status.success(),
        "reaffirm.sh failed: {}",
        String::from_utf8_lossy(&reaffirmed.stderr)
    );
    let stdout = String::from_utf8_lossy(&reaffirmed.stdout);
    assert!(
        stdout.starts_with(&format!("reaffirmed \"Shard by tenant\" ({dependent_id})")),
        "{stdout}"
    );
    let appended: Vec<Event> = events(dir)?.into_iter().skip(already).collect();
    assert_eq!(appended.len(), 1, "{appended:?}");
    assert_eq!(appended[0].actor_id, "agent:claude:plugin-reaffirm-session");
    assert_eq!(appended[0].payload["premise_id"], premise_id.as_str());

    // A refusal is not a success: exit 2 with the CLI's own reason, and nothing more is written.
    let before = events(dir)?.len();
    let refused = run_script("reaffirm", &["Shard", "by", "tenant", "--because", "Again"])?;
    assert_eq!(refused.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&refused.stderr)
            .contains("already reaffirmed after its latest change"),
        "{}",
        String::from_utf8_lossy(&refused.stderr)
    );
    assert_eq!(events(dir)?.len(), before);
    Ok(())
}
