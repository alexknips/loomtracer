// Parent module gates this file with #[cfg(test)]; repeat the marker so UBS can filter test-only assertions.
#[cfg(test)]
use super::*;
use clap::Parser;

type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;

/// What the guard makes of `hivemind <args>`: the verb's name when it guards it, `None` when the
/// verb is left out.
fn guarded_as(
    args: &[&str],
) -> std::result::Result<Option<&'static str>, Box<dyn std::error::Error>> {
    let mut argv = vec!["hivemind"];
    argv.extend_from_slice(args);
    let cli = Cli::try_parse_from(argv)?;
    Ok(CellBesideVerb::of(&cli.command).map(CellBesideVerb::name))
}

#[test]
fn every_verb_that_opens_the_ledger_is_guarded_under_its_own_name() -> TestResult {
    let cases: &[(&[&str], &str)] = &[
        (&["project", "register", "pricing"], "project register"),
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
            return Err(format!("hivemind {args:?}: expected {name:?}, got {guarded:?}").into());
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
        &["classify-queue", "list"],
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
            return Err(format!("hivemind {args:?} must not be guarded, got {guarded:?}").into());
        }
    }
    Ok(())
}

/// `tui` owns the terminal while it runs, so its notice is written to stderr once it has exited and
/// the one line it prints on stdout is left as it was.
#[test]
fn the_tui_notice_goes_to_stderr_and_leaves_stdout_alone() -> TestResult {
    let cli = Cli::try_parse_from(["hivemind", "--hivemind-dir", "/ledger/here", "tui"])?;
    let cell = LocalLedgerBesideCell {
        verb: CellBesideVerb::Tui,
        api_url: "http://cell.example.test".to_owned(),
        hivemind_dir: PathBuf::from("/ledger/here"),
    };
    let mut stderr = Vec::new();

    let stdout = with_read_notices(&cli, "tui exited".to_owned(), Some(&cell), &mut stderr)?;

    if stdout != "tui exited" {
        return Err(format!("stdout changed: {stdout:?}").into());
    }
    let stderr = String::from_utf8(stderr)?;
    if !(stderr.starts_with("notice: HIVEMIND_API_URL is set (http://cell.example.test)")
        && stderr.contains("`hivemind tui`")
        && stderr.contains("/ledger/here"))
    {
        return Err(
            format!("stderr does not name the cell, the verb and the ledger: {stderr:?}").into(),
        );
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// hivemind-i6ply: a read aimed at a folder with no ledger refuses and makes none
// ---------------------------------------------------------------------------------------------

/// What `hivemind <args>` does with the ledger it opens, `None` when it opens none.
fn ledger_use_of(
    args: &[&str],
) -> std::result::Result<Option<LedgerUse>, Box<dyn std::error::Error>> {
    let mut argv = vec!["hivemind"];
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

/// The verbs that keep making the ledger on a first run, or act on what a ledger already holds.
const NOT_READ_VERBS: &[&[&str]] = &[
    &["emit", "evidence.recorded", "--content", "seed"],
    &["ask", "How many retries does the upstream need?"],
    &["import", "documents", "notes.md"],
    &["project", "register", "pricing"],
    &["ingest", "slack-thread", "--file", "thread.json"],
    &["tenant", "create", "acme"],
    &["review", "--since", "1d"],
    &["quality-scan", "--dry-run"],
    &["disagree", "retry budget", "--reason", "Too few"],
    &["reaffirm", "retry budget", "--because", "Checked again"],
    &["restatements", "apply", "--all"],
    &["project", "use", "pricing"],
];

#[test]
fn only_the_verbs_that_just_read_are_placed_among_the_reads() -> TestResult {
    for (args, name) in READ_VERBS {
        let used = ledger_use_of(args)?;
        if used != Some(LedgerUse::Reads) {
            return Err(format!("hivemind {args:?} ({name}) is not a read: {used:?}").into());
        }
    }
    for args in NOT_READ_VERBS {
        let used = ledger_use_of(args)?;
        if used.is_none() || used == Some(LedgerUse::Reads) {
            return Err(format!("hivemind {args:?} must not be a read: {used:?}").into());
        }
    }
    Ok(())
}

/// What the refusal makes of `hivemind --hivemind-dir <dir> <args>`: its message, `None` when the
/// verb is let through. `--database-url ""` keeps a shared database in the environment out of it.
fn read_refusal(
    dir: &Path,
    args: &[&str],
) -> std::result::Result<Option<String>, Box<dyn std::error::Error>> {
    let dir = dir.display().to_string();
    let mut argv = vec!["hivemind", "--database-url", "", "--hivemind-dir", &dir];
    argv.extend_from_slice(args);
    let cli = Cli::try_parse_from(argv)?;
    Ok(refuse_read_without_ledger(&cli)
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
            let message = read_refusal(dir, args)?
                .ok_or_else(|| format!("hivemind {args:?} was let through under {dir:?}"))?;
            let shown = dir.display().to_string();
            if !(message.contains(&format!("`hivemind {name}`"))
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
fn a_read_under_a_folder_that_holds_a_ledger_is_let_through_even_when_it_holds_nothing(
) -> TestResult {
    let scratch = tempfile::tempdir()?;
    SqliteEventLedger::open(scratch.path())?;
    for (args, _) in READ_VERBS {
        if let Some(message) = read_refusal(scratch.path(), args)? {
            return Err(format!("hivemind {args:?} refused an existing ledger: {message}").into());
        }
    }
    Ok(())
}

#[test]
fn the_verbs_that_write_or_act_on_a_ledger_keep_their_first_run_behaviour() -> TestResult {
    let scratch = tempfile::tempdir()?;
    let missing = scratch.path().join("fresh");
    for args in NOT_READ_VERBS {
        if let Some(message) = read_refusal(&missing, args)? {
            return Err(format!("hivemind {args:?} is not a read, but: {message}").into());
        }
    }
    Ok(())
}

#[test]
fn a_shared_database_names_no_folder_so_a_read_is_not_refused_for_one() -> TestResult {
    let scratch = tempfile::tempdir()?;
    let dir = scratch.path().join("typo").display().to_string();
    let cli = Cli::try_parse_from([
        "hivemind",
        "--database-url",
        "postgres://nobody@127.0.0.1:9/none",
        "--hivemind-dir",
        &dir,
        "query",
        "recall",
        "retry budget",
    ])?;
    if let Err(error) = refuse_read_without_ledger(&cli) {
        return Err(format!("a configured database was refused for its folder: {error}").into());
    }
    Ok(())
}
