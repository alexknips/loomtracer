//! A truncated `query recall` hands back the cursor that reads the next page (hivemind-v3lrl).
//!
//! `recall` caps a page at `--limit` and says `truncated` when more decisions matched. The cursor
//! to continue with is the search's `next_cursor`, which counts matched records, so it has to come
//! back in `ranked.next_cursor` and in the `--summary` footer; a caller cannot work it out from the
//! items of a page, because records linked as one decision are shown as one item.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use clap::Parser;
use hivemind::cli::{run, Cli};
use serde_json::{json, Value};
use uuid::Uuid;

type TestResult<T> = std::result::Result<T, Box<dyn std::error::Error>>;

const CAPTURER: &str = "human:alice";
const QUESTION: &str = "cache eviction";

struct Scratch(PathBuf);

impl Scratch {
    fn new(label: &str) -> TestResult<Self> {
        let path = std::env::temp_dir().join(format!("hivemind-recall-{label}-{}", Uuid::new_v4()));
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

fn run_cli(dir: &Path, json: bool, args: &[&str]) -> hivemind::Result<String> {
    let mut argv = vec![
        "hivemind".to_owned(),
        "--hivemind-dir".to_owned(),
        dir.display().to_string(),
        "--actor".to_owned(),
        CAPTURER.to_owned(),
    ];
    if json {
        argv.push("--json".to_owned());
    }
    argv.extend(args.iter().map(|arg| (*arg).to_owned()));
    run(&Cli::parse_from(argv))
}

/// Records a decision with `title`, and returns its id.
fn capture(dir: &Path, title: &str) -> TestResult<String> {
    let reply: Value = serde_json::from_str(&run_cli(
        dir,
        true,
        &[
            "emit",
            "decision.capture",
            "--binds-nothing",
            "--cheap-to-undo",
            "--title",
            title,
            "--rationale",
            "The rationale is long enough for the readable floor",
            "--topic-keys",
            "cache",
            "--options",
            "LRU,FIFO",
            "--chose",
            "LRU",
            "--rests-on-assumption",
            "Entries are read far more often than they are written",
        ],
    )?)?;
    Ok(reply["value"]
        .as_str()
        .ok_or("capture returns the decision id")?
        .to_owned())
}

/// `query recall` for `QUESTION`, as `--json`: the `data.ranked` of the answer.
fn ranked_page(dir: &Path, limit: usize, cursor: Option<&str>) -> TestResult<Value> {
    let limit = limit.to_string();
    let mut args = vec!["query", "recall", QUESTION, "--limit", &limit];
    if let Some(cursor) = cursor {
        args.extend(["--cursor", cursor]);
    }
    let mut answer: Value = serde_json::from_str(&run_cli(dir, true, &args)?)?;
    Ok(answer["data"]["ranked"].take())
}

fn item_ids(page: &Value) -> TestResult<Vec<String>> {
    Ok(page["items"]
        .as_array()
        .ok_or("ranked.items is a list")?
        .iter()
        .filter_map(|item| item["decision"]["id"].as_str().map(str::to_owned))
        .collect())
}

/// What `ranked.next_cursor` says, failing when the page has no such field at all: a missing
/// field and a null one both index as `Null`, and only the null one means "last page".
fn next_cursor(page: &Value) -> TestResult<Option<String>> {
    let field = page
        .as_object()
        .and_then(|ranked| ranked.get("next_cursor"))
        .ok_or("ranked carries next_cursor")?;
    Ok(field.as_str().map(str::to_owned))
}

#[test]
fn a_truncated_recall_is_walked_page_by_page_with_its_cursor() -> TestResult<()> {
    let scratch = Scratch::new("walk")?;
    let dir = scratch.path();
    let mut recorded = HashSet::new();
    for number in 1..=7 {
        recorded.insert(capture(
            dir,
            &format!("Cache eviction rule {number} is LRU"),
        )?);
    }

    let first = ranked_page(dir, 3, None)?;
    assert_eq!(first["truncated"], json!(true));
    assert_eq!(first["total_matches"], json!(7));
    assert_eq!(item_ids(&first)?.len(), 3);
    let second_cursor = next_cursor(&first)?.ok_or("a truncated page names where to continue")?;

    let second = ranked_page(dir, 3, Some(&second_cursor))?;
    assert_eq!(second["truncated"], json!(true));
    assert_eq!(item_ids(&second)?.len(), 3);
    let third_cursor = next_cursor(&second)?.ok_or("a truncated page names where to continue")?;
    assert_ne!(third_cursor, second_cursor, "the cursor moves on");

    let third = ranked_page(dir, 3, Some(&third_cursor))?;
    assert_eq!(third["truncated"], json!(false));
    assert_eq!(item_ids(&third)?.len(), 1);
    assert_eq!(next_cursor(&third)?, None, "the last page has no cursor");

    let walked: Vec<String> = [&first, &second, &third]
        .into_iter()
        .map(item_ids)
        .collect::<TestResult<Vec<_>>>()?
        .concat();
    assert_eq!(walked.len(), 7, "no decision is repeated: {walked:?}");
    assert_eq!(walked.into_iter().collect::<HashSet<_>>(), recorded);

    // The text summary closes a truncated page with the cursor, and says nothing on the last.
    let summary = run_cli(
        dir,
        false,
        &["query", "recall", QUESTION, "--limit", "3", "--summary"],
    )?;
    assert!(
        summary.ends_with(&format!("truncated=true next_cursor={second_cursor}")),
        "the summary names the cursor, got:\n{summary}"
    );
    let last_summary = run_cli(
        dir,
        false,
        &[
            "query",
            "recall",
            QUESTION,
            "--limit",
            "3",
            "--cursor",
            &third_cursor,
            "--summary",
        ],
    )?;
    assert!(
        !last_summary.contains("truncated") && !last_summary.contains("next_cursor"),
        "the last page is not truncated, got:\n{last_summary}"
    );
    Ok(())
}

/// A decision recorded twice and linked `SAME_AS` is one item, so a page of three records shows two
/// items. The cursor still counts the three records read: counting the items would read the third
/// record again on the next page.
#[test]
fn the_cursor_counts_records_when_linked_records_are_shown_as_one_item() -> TestResult<()> {
    let scratch = Scratch::new("linked")?;
    let dir = scratch.path();
    let submit = |session: &str, captures: Value| -> TestResult<()> {
        let path = dir.join(format!("captures-{session}.json"));
        fs::write(&path, captures.to_string())?;
        run_cli(
            dir,
            true,
            &[
                "emit",
                "ingest.batch_classified",
                "--captures",
                path.to_str().ok_or("utf-8 captures path")?,
                "--agent-tool",
                "claude",
                "--agent-session",
                session,
                "--classifier-model",
                "claude-haiku-4-5-20251001",
            ],
        )?;
        Ok(())
    };
    // The two records of the linked decision say the question in their titles; the others only in
    // their rationale, so they come after them.
    let capture_json = |title: &str, rationale: &str, restates_id: Option<&str>| {
        let mut capture = json!({
            "kind": "decision",
            "title": title,
            "rationale": rationale,
            "topic_keys": ["cache"],
            "evidence_ids": [],
            "options": ["LRU", "FIFO"],
            "chosen_option": "LRU",
            "extraction_confidence": 0.9
        });
        if let Some(restates_id) = restates_id {
            capture["restates_id"] = json!(restates_id);
        }
        capture
    };

    submit(
        "first",
        json!([capture_json(
            "Cache eviction is least-recently-used",
            "Entries are read far more often than they are written",
            None
        )]),
    )?;
    let first_id = item_ids(&ranked_page(dir, 5, None)?)?
        .pop()
        .ok_or("the first record is recalled")?;
    submit(
        "second",
        json!([
            capture_json(
                "Cache eviction stays least-recently-used",
                "Entries are read far more often than they are written",
                Some(&first_id)
            ),
            capture_json(
                "Pool size follows the core count",
                "Cache eviction needs no pool of its own, so the pool is sized by cores",
                None
            ),
            capture_json(
                "Retries stop after three attempts",
                "A failed fill is retried, never evicted: cache eviction is not a retry",
                None
            ),
        ]),
    )?;

    let first = ranked_page(dir, 3, None)?;
    assert_eq!(first["total_matches"], json!(4), "four records matched");
    let first_items = item_ids(&first)?;
    assert_eq!(
        first_items.len(),
        2,
        "three records, two linked: {first_items:?}"
    );
    assert_eq!(first_items[0], first_id, "the earliest record is shown");
    assert_eq!(
        first["items"][0]["also_recorded_as"]
            .as_array()
            .map(Vec::len),
        Some(1)
    );
    assert_eq!(first["truncated"], json!(true));
    let cursor = next_cursor(&first)?.ok_or("a truncated page names where to continue")?;
    assert_eq!(cursor, "3", "three records were read, not two items");

    let second = ranked_page(dir, 3, Some(&cursor))?;
    let second_items = item_ids(&second)?;
    assert_eq!(
        second_items.len(),
        1,
        "one record is left: {second_items:?}"
    );
    assert!(
        !first_items.contains(&second_items[0]),
        "the next page repeats nothing"
    );
    assert_eq!(second["truncated"], json!(false));
    assert_eq!(next_cursor(&second)?, None);
    Ok(())
}
