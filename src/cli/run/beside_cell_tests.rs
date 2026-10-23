// Parent module gates this file with #[cfg(test)]; repeat the marker so UBS can filter test-only assertions.
#[cfg(test)]
use super::*;
use clap::Parser;

type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;

/// What the guard makes of `loomtracer <args>`: the verb's name when it guards it, `None` when the
/// verb is left out.
fn guarded_as(
    args: &[&str],
) -> std::result::Result<Option<&'static str>, Box<dyn std::error::Error>> {
    let mut argv = vec!["loomtracer"];
    argv.extend_from_slice(args);
    let cli = Cli::try_parse_from(argv)?;
    Ok(CellBesideVerb::of(&cli.command).map(CellBesideVerb::name))
}

#[test]
fn every_verb_that_opens_the_ledger_is_guarded_under_its_own_name() -> TestResult {
    let cases: &[(&[&str], &str)] = &[
        (&["project", "register", "pricing"], "project register"),
        (
            &["project", "rename", "pricing", "billing"],
            "project rename",
        ),
        (
            &[
                "project", "link", "--from", "a-app", "--to", "b-app", "--kind", "part_of",
            ],
            "project link",
        ),
        (
            &[
                "project", "unlink", "--from", "a-app", "--to", "b-app", "--kind", "part_of",
            ],
            "project unlink",
        ),
        (
            &[
                "project", "anchor", "--handle", "pricing", "--kind", "folder", "--value", "/x",
            ],
            "project anchor",
        ),
        (
            &["project", "declare-topic", "pricing", "seats"],
            "project declare-topic",
        ),
        (&["project", "list"], "project list"),
        (&["project", "show", "pricing"], "project show"),
        (&["project", "decisions", "pricing"], "project decisions"),
        (&["project", "use", "pricing"], "project use"),
        (
            &["ingest", "slack-thread", "--file", "thread.json"],
            "ingest slack-thread",
        ),
        (&["tui"], "tui"),
        (
            &["reaffirm", "retry budget", "--because", "Checked again"],
            "reaffirm",
        ),
        (&["reexamine"], "reexamine"),
        (&["tenant", "create", "acme"], "tenant create"),
        // The verbs the earlier rounds guarded stay guarded.
        (&["query", "recent_decisions", "--since", "7d"], "query"),
        (&["restatements", "propose"], "restatements propose"),
        (&["restatements", "apply", "--all"], "restatements apply"),
        (&["import", "documents", "notes.md"], "import"),
    ];
    for (args, name) in cases {
        let guarded = guarded_as(args)?;
        if guarded != Some(*name) {
            return Err(format!("loomtracer {args:?}: expected {name:?}, got {guarded:?}").into());
        }
    }
    Ok(())
}

#[test]
fn the_verbs_left_out_on_purpose_are_not_guarded() -> TestResult {
    let cases: &[&[&str]] = &[
        &["quickstart"],
        &["serve"],
        &["mcp"],
        &[
            "migrate",
            "--to",
            "postgres://nobody@127.0.0.1:9/none",
            "--to-tenant",
            "acme",
            "--dry-run",
        ],
        &["migrate-storage", "--dry-run"],
        &["connector", "auth", "gdocs"],
        &["import", "prepare-documents", "notes.md"],
        &[
            "slack-app",
            "manifest",
            "--request-url",
            "https://example.test/slack",
        ],
        // These two open only the current-project file; the same verbs with a handle or without
        // `--current` open the ledger and are guarded above.
        &["project", "show", "--current"],
        &["project", "use", "--clear"],
        &["project", "use"],
    ];
    for args in cases {
        let guarded = guarded_as(args)?;
        if guarded.is_some() {
            return Err(format!("loomtracer {args:?} must not be guarded, got {guarded:?}").into());
        }
    }
    Ok(())
}

/// `tui` owns the terminal while it runs, so its notice is written to stderr once it has exited and
/// the one line it prints on stdout is left as it was.
#[test]
fn the_tui_notice_goes_to_stderr_and_leaves_stdout_alone() -> TestResult {
    let cli = Cli::try_parse_from(["loomtracer", "--loomtracer-dir", "/ledger/here", "tui"])?;
    let cell = LocalLedgerBesideCell {
        verb: CellBesideVerb::Tui,
        api_url: "http://cell.example.test".to_owned(),
        loomtracer_dir: PathBuf::from("/ledger/here"),
    };
    let mut stderr = Vec::new();

    let stdout = with_read_notices(&cli, "tui exited".to_owned(), Some(&cell), &mut stderr)?;

    if stdout != "tui exited" {
        return Err(format!("stdout changed: {stdout:?}").into());
    }
    let stderr = String::from_utf8(stderr)?;
    if !(stderr.starts_with("notice: LOOMTRACER_API_URL is set (http://cell.example.test)")
        && stderr.contains("`loomtracer tui`")
        && stderr.contains("/ledger/here"))
    {
        return Err(
            format!("stderr does not name the cell, the verb and the ledger: {stderr:?}").into(),
        );
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// hivemind-i6ply, hivemind-5t1uk: a verb that reads or acts on a ledger, aimed at a folder with
// none, refuses and makes none
// ---------------------------------------------------------------------------------------------

/// What `loomtracer <args>` does with the ledger it opens, `None` when it opens none.
fn ledger_use_of(
    args: &[&str],
) -> std::result::Result<Option<LedgerUse>, Box<dyn std::error::Error>> {
    let mut argv = vec!["loomtracer"];
    argv.extend_from_slice(args);
    let cli = Cli::try_parse_from(argv)?;
    Ok(CellBesideVerb::of(&cli.command).map(CellBesideVerb::ledger_use))
}

/// Every verb that only answers from the ledger, one invocation each. `query` stands for all of
/// its subcommands, so the ones people ask with (`why`, `verify`, `recall`, `search`,
/// `situational`) are listed.
const READ_VERBS: &[(&[&str], &str)] = &[
    (&["query", "why", "retry budget"], "query"),
    (&["query", "verify", "retry budget"], "query"),
    (&["query", "recall", "retry budget"], "query"),
    (&["query", "search", "--q", "retry"], "query"),
    (
        &["query", "situational", "--paths", "src/retry.rs"],
        "query",
    ),
    (&["query", "recent_decisions", "--since", "7d"], "query"),
    (&["digest", "--window", "1d"], "digest"),
    (
        &["export", "--format", "markdown", "--out", "out"],
        "export",
    ),
    (&["dump"], "dump"),
    (&["map"], "map"),
    (&["restatements", "propose"], "restatements propose"),
    (&["reexamine"], "reexamine"),
    (&["project", "list"], "project list"),
    (&["project", "show", "pricing"], "project show"),
    (&["project", "decisions", "pricing"], "project decisions"),
    (&["tui"], "tui"),
];

/// Every verb that acts on a decision or project a ledger already holds, one invocation each.
/// None of them can succeed on a ledger that does not exist, so a folder with none is refused.
const ACT_ON_VERBS: &[(&[&str], &str)] = &[
    (
        &["disagree", "retry budget", "--reason", "Too few"],
        "disagree",
    ),
    (
        &[
            "supersede",
            "retry budget",
            "--title",
            "Retry five times",
            "--rationale",
            "Three retries lost two writes a week, so allow five.",
            "--bet",
        ],
        "supersede",
    ),
    (&["move", "retry budget", "--to", "pricing"], "move"),
    (
        &["retitle", "retry budget", "--to", "Retry five times"],
        "retitle",
    ),
    (&["ground", "retry budget", "--bet"], "ground"),
    (
        &["reaffirm", "retry budget", "--because", "Checked again"],
        "reaffirm",
    ),
    (&["review", "--since", "1d"], "review"),
    (&["quality-scan", "--dry-run"], "quality-scan"),
    (&["restatements", "apply", "--all"], "restatements apply"),
    (&["project", "rename", "a-app", "b-app"], "project rename"),
    (
        &[
            "project", "link", "--from", "a-app", "--to", "b-app", "--kind", "part_of",
        ],
        "project link",
    ),
    (
        &[
            "project", "unlink", "--from", "a-app", "--to", "b-app", "--kind", "part_of",
        ],
        "project unlink",
    ),
    (
        &[
            "project", "anchor", "--handle", "pricing", "--kind", "folder", "--value", "/x",
        ],
        "project anchor",
    ),
    (
        &["project", "declare-topic", "pricing", "seats"],
        "project declare-topic",
    ),
    (&["project", "use", "pricing"], "project use"),
];

/// The verbs that record the first event of a ledger, and so keep making it on a first run.
const WRITE_VERBS: &[&[&str]] = &[
    &["emit", "evidence.recorded", "--content", "seed"],
    &["ask", "How many retries does the upstream need?"],
    &["import", "documents", "notes.md"],
    &["project", "register", "pricing"],
    &["ingest", "slack-thread", "--file", "thread.json"],
    &["tenant", "create", "acme"],
];

#[test]
fn every_verb_is_placed_by_what_it_does_with_the_ledger() -> TestResult {
    for (args, name) in READ_VERBS {
        let used = ledger_use_of(args)?;
        if used != Some(LedgerUse::Reads) {
            return Err(format!("loomtracer {args:?} ({name}) is not a read: {used:?}").into());
        }
    }
    for (args, name) in ACT_ON_VERBS {
        let used = ledger_use_of(args)?;
        if used != Some(LedgerUse::ReadsAndWrites) {
            return Err(
                format!("loomtracer {args:?} ({name}) does not act on a ledger: {used:?}").into(),
            );
        }
    }
    for args in WRITE_VERBS {
        let used = ledger_use_of(args)?;
        if used != Some(LedgerUse::WritesTo) {
            return Err(
                format!("loomtracer {args:?} must be able to start a ledger: {used:?}").into(),
            );
        }
    }
    Ok(())
}

/// `classify-queue` opens the ledger under `--loomtracer-dir` only while `LOOMTRACER_API_URL` is
/// unset (hivemind-odmnc): it is placed by what it does with that ledger, so a folder with none is
/// refused, and it is the one verb the cell guard leaves alone, because with the variable set it
/// asks that server instead.
#[test]
fn classify_queue_is_placed_by_its_ledger_use_and_left_to_the_cell() -> TestResult {
    let list: &[&str] = &["classify-queue", "list"];
    let submit: &[&str] = &[
        "classify-queue",
        "submit",
        "--batch-id",
        "b-1",
        "--captures",
        "[]",
    ];
    if ledger_use_of(list)? != Some(LedgerUse::Reads) {
        return Err("classify-queue list is a read of the local ledger".into());
    }
    if ledger_use_of(submit)? != Some(LedgerUse::ReadsAndWrites) {
        return Err("classify-queue submit acts on batches the local ledger holds".into());
    }
    for (args, name) in [
        (list, "classify-queue list"),
        (submit, "classify-queue submit"),
    ] {
        if guarded_as(args)? != Some(name) {
            return Err(format!("loomtracer {args:?} is not named {name:?}").into());
        }
        let mut argv = vec!["loomtracer"];
        argv.extend_from_slice(args);
        let cli = Cli::try_parse_from(argv)?;
        let honours = CellBesideVerb::of(&cli.command).is_some_and(CellBesideVerb::honours_api_url);
        if !honours {
            return Err(format!("loomtracer {args:?} must be left to the cell guard").into());
        }
    }
    // No other verb is: each of them ignores the variable and is guarded beside a cell.
    for (args, name) in READ_VERBS.iter().chain(ACT_ON_VERBS) {
        let mut argv = vec!["loomtracer"];
        argv.extend_from_slice(args);
        let cli = Cli::try_parse_from(argv)?;
        if CellBesideVerb::of(&cli.command).is_some_and(CellBesideVerb::honours_api_url) {
            return Err(format!("loomtracer {args:?} ({name}) honours LOOMTRACER_API_URL").into());
        }
    }
    Ok(())
}

/// What the refusal makes of `loomtracer --loomtracer-dir <dir> <args>`: its message, `None` when the
/// verb is let through. `--database-url ""` keeps a shared database in the environment out of it.
fn ledger_refusal(
    dir: &Path,
    args: &[&str],
) -> std::result::Result<Option<String>, Box<dyn std::error::Error>> {
    let dir = dir.display().to_string();
    let mut argv = vec!["loomtracer", "--database-url", "", "--loomtracer-dir", &dir];
    argv.extend_from_slice(args);
    let cli = Cli::try_parse_from(argv)?;
    Ok(refuse_verb_without_ledger(&cli)
        .err()
        .map(|error| error.to_string()))
}

#[test]
fn a_read_with_no_ledger_under_the_folder_is_refused_naming_the_folder_and_makes_none() -> TestResult
{
    let scratch = tempfile::tempdir()?;
    let missing = scratch.path().join("typo");
    let empty = scratch.path().join("empty");
    std::fs::create_dir(&empty)?;
    for (args, name) in READ_VERBS {
        for dir in [&missing, &empty] {
            let message = ledger_refusal(dir, args)?
                .ok_or_else(|| format!("loomtracer {args:?} was let through under {dir:?}"))?;
            let shown = dir.display().to_string();
            if !(message.contains(&format!("`loomtracer {name}` reads a local ledger"))
                && message.contains(&shown)
                && message.contains("none was created"))
            {
                return Err(format!(
                    "the refusal for {args:?} must name the verb, {shown} and that nothing was made: {message}"
                )
                .into());
            }
        }
    }
    if missing.exists() {
        return Err("a refused read made the folder".into());
    }
    if std::fs::read_dir(&empty)?.next().is_some() {
        return Err("a refused read put something in the folder".into());
    }
    Ok(())
}

#[test]
fn a_verb_that_acts_on_a_ledger_with_none_under_the_folder_is_refused_naming_the_folder_and_makes_none(
) -> TestResult {
    let scratch = tempfile::tempdir()?;
    let missing = scratch.path().join("typo");
    let empty = scratch.path().join("empty");
    std::fs::create_dir(&empty)?;
    for (args, name) in ACT_ON_VERBS {
        for dir in [&missing, &empty] {
            let message = ledger_refusal(dir, args)?
                .ok_or_else(|| format!("loomtracer {args:?} was let through under {dir:?}"))?;
            let shown = dir.display().to_string();
            if !(message.contains(&format!("`loomtracer {name}` acts on what a local ledger"))
                && message.contains(&shown)
                && message.contains("nothing was recorded and none was created")
                && !message.contains("reads a local ledger"))
            {
                return Err(format!(
                    "the refusal for {args:?} must name the verb, {shown} and that nothing was recorded or made: {message}"
                )
                .into());
            }
        }
    }
    if missing.exists() {
        return Err("a refused verb made the folder".into());
    }
    if std::fs::read_dir(&empty)?.next().is_some() {
        return Err("a refused verb put something in the folder".into());
    }
    Ok(())
}

#[test]
fn a_verb_under_a_folder_that_holds_a_ledger_is_let_through_even_when_it_holds_nothing(
) -> TestResult {
    let scratch = tempfile::tempdir()?;
    SqliteEventLedger::open(scratch.path())?;
    let reads = READ_VERBS.iter().map(|(args, _)| *args);
    let acts = ACT_ON_VERBS.iter().map(|(args, _)| *args);
    for args in reads.chain(acts) {
        if let Some(message) = ledger_refusal(scratch.path(), args)? {
            return Err(
                format!("loomtracer {args:?} refused an existing ledger: {message}").into(),
            );
        }
    }
    Ok(())
}

#[test]
fn the_verbs_that_write_keep_their_first_run_behaviour() -> TestResult {
    let scratch = tempfile::tempdir()?;
    let missing = scratch.path().join("fresh");
    for args in WRITE_VERBS {
        if let Some(message) = ledger_refusal(&missing, args)? {
            return Err(format!("loomtracer {args:?} starts a ledger, but: {message}").into());
        }
    }
    Ok(())
}

#[test]
fn a_shared_database_names_no_folder_so_a_read_is_not_refused_for_one() -> TestResult {
    let scratch = tempfile::tempdir()?;
    let dir = scratch.path().join("typo").display().to_string();
    let cli = Cli::try_parse_from([
        "loomtracer",
        "--database-url",
        "postgres://nobody@127.0.0.1:9/none",
        "--loomtracer-dir",
        &dir,
        "query",
        "recall",
        "retry budget",
    ])?;
    if let Err(error) = refuse_verb_without_ledger(&cli) {
        return Err(format!("a configured database was refused for its folder: {error}").into());
    }
    Ok(())
}
