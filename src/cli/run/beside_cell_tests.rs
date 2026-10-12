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
