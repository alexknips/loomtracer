use super::*;

/// The tool names under `## Write tools` and under `## Read tools` in a generated page.
fn tools_by_section(page: &str) -> (Vec<String>, Vec<String>) {
    let (mut writes, mut reads) = (Vec::new(), Vec::new());
    let mut section = "";
    for line in page.lines() {
        if let Some(heading) = line.strip_prefix("## ") {
            section = heading;
        } else if let Some(name) = line
            .strip_prefix("### `")
            .and_then(|rest| rest.strip_suffix('`'))
        {
            match section {
                "Write tools" => writes.push(name.to_owned()),
                "Read tools" => reads.push(name.to_owned()),
                _ => {}
            }
        }
    }
    (writes, reads)
}

#[test]
fn the_write_section_is_the_servers_write_set() {
    let (mut writes, reads) = tools_by_section(&generate_mcp_reference());

    // The three tools the hand-kept list once missed.
    for name in [
        "retitle_decision",
        "request_decision",
        "classify_queue_submit",
    ] {
        assert!(
            writes.iter().any(|w| w == name),
            "{name} is not a write tool"
        );
        assert!(
            !reads.iter().any(|r| r == name),
            "{name} is listed as a read tool"
        );
    }

    let mut server_writes: Vec<String> =
        write_tool_names().into_iter().map(str::to_owned).collect();
    writes.sort();
    server_writes.sort();
    assert_eq!(
        writes, server_writes,
        "the page's Write tools differ from the server's write set"
    );
    assert_eq!(
        writes.len() + reads.len(),
        tool_definitions().len(),
        "a tool is on neither side of the page, or on both"
    );
}

/// A cli.md heading for every verb, and for every emit and query subcommand, the CLI has.
fn complete_cli_md(root: &Command) -> String {
    let mut md = String::new();
    for top in root.get_subcommands() {
        writeln!(md, "### `{}`", top.get_name()).unwrap();
        if matches!(top.get_name(), "emit" | "query") {
            for sub in top.get_subcommands() {
                writeln!(md, "### `{} {}`", top.get_name(), sub.get_name()).unwrap();
            }
        }
    }
    md
}

#[test]
fn a_cli_md_with_a_section_for_every_verb_is_complete() {
    let root = Cli::command();
    assert_eq!(
        missing_cli_subcommands(&root, &complete_cli_md(&root)),
        Vec::<String>::new()
    );
}

#[test]
fn a_top_level_verb_without_a_section_is_reported() {
    let root = Cli::command();
    let without_reaffirm = complete_cli_md(&root).replace("### `reaffirm`\n", "");
    assert_eq!(
        missing_cli_subcommands(&root, &without_reaffirm),
        ["reaffirm"]
    );

    // Mentioning the verb in prose is not documenting it.
    let mentioned = format!("{without_reaffirm}\nRun `loomtracer reaffirm <id>` to confirm.\n");
    assert_eq!(missing_cli_subcommands(&root, &mentioned), ["reaffirm"]);
}

#[test]
fn a_verb_named_in_a_shared_heading_is_documented() {
    let root = Cli::command();
    let shared = complete_cli_md(&root)
        .replace("### `reaffirm`\n", "")
        .replace("### `ground`\n", "### `ground` / `reaffirm`\n");
    assert_eq!(
        missing_cli_subcommands(&root, &shared),
        Vec::<String>::new()
    );
}
