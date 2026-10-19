//! Every `hivemind ...` command the README, docs and plugin files show still parses (hivemind-gq7dm).
//!
//! Doc drift is the commonest bug class here: a flag is renamed or a verb retired, and the example
//! that taught it keeps teaching it until a reader hits exit 2. This test reads each Markdown file
//! that teaches the CLI, pulls out every command that starts with `hivemind`, and gives it to the
//! real parser, `hivemind::cli::Cli::try_parse_from`. Nothing is run and no ledger is touched.
//!
//! Two bars:
//! - A fenced shell example is a whole invocation and must parse.
//! - An inline `code span` in prose, and any command that elides arguments (`...`, `[--flag N]`),
//!   is a sketch of one. It must name a real verb and real flags; what it leaves out is not held
//!   against it.
//!
//! A `<placeholder>`, `$VAR`, `A|B` or `N` stands for a value the reader supplies, so it is swapped
//! for one the parser accepts and the rest of the command is still checked.
//!
//! What a doc shows on purpose and the CLI does not do (a design doc sketching a flag, a bead that
//! has not landed its doc fix) goes in `ALLOWED` with a reason. An entry that no longer excuses
//! anything fails the test, so the list only shrinks.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use clap::error::{ContextKind, ContextValue, ErrorKind};
use clap::Parser;
use hivemind::cli::Cli;
use hivemind::mcp::tool_definitions;

/// A command shown in a doc that the CLI does not accept, and why that is allowed for now.
/// `command` is a prefix of the example as it reads in the file, whitespace collapsed.
struct Allowed {
    file: &'static str,
    command: &'static str,
    why: &'static str,
}

const ALLOWED: &[Allowed] = &[
    // Design records. They name what was proposed at the time; the shipped names differ.
    Allowed {
        file: "docs/AGENT_FLUENT_QUERYING.md",
        command: "hivemind query context",
        why: "the old working name of `query situational`, named as such beside it",
    },
    Allowed {
        file: "docs/AGENT_FLUENT_QUERYING.md",
        command: "hivemind query review",
        why: "the proposed alias of get_decision_outcome; it shipped as `verify`",
    },
    // Design docs that show planned flags as if they were current (hivemind-t97qx).
    Allowed {
        file: "docs/AUTH_MODEL.md",
        command: "hivemind --remote",
        why: "hivemind-t97qx: --remote and --token-env are not built",
    },
    Allowed {
        file: "docs/AUTH_MODEL.md",
        command: "hivemind mcp --remote",
        why: "hivemind-t97qx: --remote and --token-env are not built",
    },
    Allowed {
        file: "docs/SEARCH_DESIGN.md",
        command: "hivemind query search --q <text>",
        why: "hivemind-t97qx: the planned search flags are not built",
    },
    Allowed {
        file: "docs/SEARCH_DESIGN.md",
        command: "hivemind query search --status contested",
        why: "hivemind-t97qx: the planned search flags are not built",
    },
    Allowed {
        file: "docs/SEARCH_DESIGN.md",
        command: "hivemind query search --premised-on-hypothesis-id",
        why: "hivemind-t97qx: the planned search flags are not built",
    },
    Allowed {
        file: "docs/SEARCH_DESIGN.md",
        command: "hivemind query search --q \"cache eviction\" --field",
        why: "hivemind-t97qx: the planned search flags are not built",
    },
    // Import docs that describe a retired verb and a Confluence connector (hivemind-htomw).
    Allowed {
        file: "docs/INGESTION_CONNECTORS.md",
        command: "hivemind connector auth confluence",
        why: "hivemind-htomw: there is no Confluence connector auth",
    },
    Allowed {
        file: "docs/TEXT_IMPORT_AND_DIFF_SEMANTICS.md",
        command: "hivemind suggest",
        why: "hivemind-htomw: `suggest` is retired",
    },
    Allowed {
        file: "docs/TEXT_IMPORT_AND_DIFF_SEMANTICS.md",
        command: "hivemind --json suggest",
        why: "hivemind-htomw: `suggest` is retired",
    },
    Allowed {
        file: "docs/TEXT_IMPORT_AND_DIFF_SEMANTICS.md",
        command: "hivemind --actor reviewer:alice --json suggest",
        why: "hivemind-htomw: `suggest` is retired",
    },
];

/// Info strings of fenced blocks that hold shell commands. Other blocks (json, yaml, rust, ...)
/// are not commands even when a line starts with `hivemind`.
const SHELL_FENCES: &[&str] = &["", "bash", "sh", "shell", "zsh", "console", "shell-session"];

/// Release history names flags and verbs as they were when each version shipped.
const SKIPPED_FILES: &[&str] = &["CHANGELOG.md"];

const MAX_PLACEHOLDER_RETRIES: usize = 16;

struct Example {
    file: String,
    line: usize,
    text: String,
    fenced: bool,
}

struct Word {
    text: String,
    quoted: bool,
}

struct Arg {
    text: String,
    placeholder: bool,
}

// ---------------------------------------------------------------------------------------------
// Finding the files and the examples in them
// ---------------------------------------------------------------------------------------------

fn markdown_files(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for entry in fs::read_dir(root).expect("read the repo root") {
        let entry = entry.expect("read a repo root entry");
        // `file_type` does not follow links, so CLAUDE.md (a link to AGENTS.md) is not read twice.
        let is_file = entry.file_type().map(|t| t.is_file()).unwrap_or(false);
        if is_file && is_markdown(&entry.path()) {
            files.push(entry.path());
        }
    }
    for dir in ["docs", "plugins"] {
        collect_markdown(&root.join(dir), &mut files);
    }
    files.retain(|path| {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        path.parent() != Some(root) || !SKIPPED_FILES.contains(&name)
    });
    files.sort();
    files
}

fn collect_markdown(dir: &Path, files: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries {
        let entry = entry.expect("read a directory entry");
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_dir() {
            collect_markdown(&entry.path(), files);
        } else if kind.is_file() && is_markdown(&entry.path()) {
            files.push(entry.path());
        }
    }
}

fn is_markdown(path: &Path) -> bool {
    path.extension().is_some_and(|ext| ext == "md")
}

fn examples_in(file: &str, text: &str) -> Vec<Example> {
    struct Fence {
        marker: char,
        len: usize,
        shell: bool,
        lines: Vec<(usize, String)>,
    }

    let mut found = Vec::new();
    let mut fence: Option<Fence> = None;
    let mut paragraph: Vec<(usize, &str)> = Vec::new();

    for (index, raw) in text.lines().enumerate() {
        let line = index + 1;
        let trimmed = raw.trim();
        if let Some(open) = fence.as_mut() {
            let closes = trimmed.len() >= open.len && trimmed.chars().all(|c| c == open.marker);
            if closes {
                let done = fence.take().expect("a fence is open");
                if done.shell {
                    for (line, command) in fenced_commands(&done.lines) {
                        found.push(Example {
                            file: file.to_string(),
                            line,
                            text: command,
                            fenced: true,
                        });
                    }
                }
            } else {
                open.lines.push((line, trimmed.to_string()));
            }
            continue;
        }
        if let Some(marker) = ['`', '~']
            .into_iter()
            .find(|m| trimmed.starts_with(&m.to_string().repeat(3)))
        {
            push_inline(file, &mut paragraph, &mut found);
            let len = trimmed.chars().take_while(|c| *c == marker).count();
            let info = trimmed[len..].split_whitespace().next().unwrap_or("");
            fence = Some(Fence {
                marker,
                len,
                shell: SHELL_FENCES.contains(&info),
                lines: Vec::new(),
            });
        } else if trimmed.is_empty() {
            push_inline(file, &mut paragraph, &mut found);
        } else {
            paragraph.push((line, trimmed));
        }
    }
    push_inline(file, &mut paragraph, &mut found);
    found
}

/// The commands in a shell block: each line that starts with `hivemind`, its `\` continuations
/// joined on. A leading `$ ` prompt is dropped.
fn fenced_commands(lines: &[(usize, String)]) -> Vec<(usize, String)> {
    let mut commands = Vec::new();
    let mut at = 0;
    while at < lines.len() {
        let (line, text) = &lines[at];
        let mut command = text.strip_prefix("$ ").unwrap_or(text).to_string();
        at += 1;
        if !starts_with_hivemind(&command) {
            continue;
        }
        while command.ends_with('\\') && at < lines.len() {
            command.pop();
            command.push(' ');
            command.push_str(&lines[at].1);
            at += 1;
        }
        commands.push((*line, collapse(&command)));
    }
    commands
}

fn push_inline(file: &str, paragraph: &mut Vec<(usize, &str)>, found: &mut Vec<Example>) {
    for (line, span) in code_spans(paragraph) {
        if starts_with_hivemind(&span) {
            found.push(Example {
                file: file.to_string(),
                line,
                text: span,
                fenced: false,
            });
        }
    }
    paragraph.clear();
}

/// The backtick code spans of one paragraph, each with the line it opens on. A span may run over
/// a line break, so the paragraph is read as a whole.
fn code_spans(paragraph: &[(usize, &str)]) -> Vec<(usize, String)> {
    let mut joined = String::new();
    let mut starts = Vec::new();
    for (line, text) in paragraph {
        starts.push((joined.len(), *line));
        joined.push_str(text);
        joined.push('\n');
    }
    let bytes = joined.as_bytes();
    let mut spans = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] != b'`' {
            at += 1;
            continue;
        }
        let open = at;
        while at < bytes.len() && bytes[at] == b'`' {
            at += 1;
        }
        let run = at - open;
        let content = at;
        let mut scan = at;
        while scan < bytes.len() {
            if bytes[scan] != b'`' {
                scan += 1;
                continue;
            }
            let close = scan;
            while scan < bytes.len() && bytes[scan] == b'`' {
                scan += 1;
            }
            if scan - close == run {
                let line = starts
                    .iter()
                    .rev()
                    .find(|(offset, _)| *offset <= open)
                    .map_or(0, |(_, line)| *line);
                spans.push((line, collapse(&joined[content..close].replace("\\|", "|"))));
                at = scan;
                break;
            }
        }
    }
    spans
}

fn starts_with_hivemind(text: &str) -> bool {
    text.strip_prefix("hivemind")
        .is_some_and(|rest| rest.starts_with(char::is_whitespace))
}

fn collapse(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

// ---------------------------------------------------------------------------------------------
// Reading one command the way a shell would
// ---------------------------------------------------------------------------------------------

/// Splits a command into words, stopping at the first pipe, redirect, `;`, `&&` or comment.
fn lex(text: &str) -> Result<Vec<Word>, String> {
    let chars: Vec<char> = text.chars().collect();
    let mut words = Vec::new();
    let mut at = 0;
    while at < chars.len() {
        if chars[at].is_whitespace() {
            at += 1;
            continue;
        }
        let next_is_alpha = chars.get(at + 1).is_some_and(|c| c.is_alphabetic());
        match chars[at] {
            '|' | '&' | ';' | '>' | '#' => break,
            '<' if !next_is_alpha => break,
            c if c.is_ascii_digit() && chars.get(at + 1) == Some(&'>') => break,
            _ => {}
        }
        let mut word = Word {
            text: String::new(),
            quoted: false,
        };
        while at < chars.len() && !chars[at].is_whitespace() {
            let next_is_alpha = chars.get(at + 1).is_some_and(|c| c.is_alphabetic());
            match chars[at] {
                '\'' => {
                    word.quoted = true;
                    at += 1;
                    while chars.get(at).ok_or("an unterminated ' quote")? != &'\'' {
                        word.text.push(chars[at]);
                        at += 1;
                    }
                    at += 1;
                }
                '"' => {
                    word.quoted = true;
                    at += 1;
                    loop {
                        match chars.get(at).ok_or("an unterminated \" quote")? {
                            '"' => break,
                            '\\' if matches!(chars.get(at + 1), Some('"' | '\\' | '$' | '`')) => {
                                word.text.push(chars[at + 1]);
                                at += 2;
                            }
                            c => {
                                word.text.push(*c);
                                at += 1;
                            }
                        }
                    }
                    at += 1;
                }
                '\\' => {
                    if let Some(c) = chars.get(at + 1) {
                        word.text.push(*c);
                    }
                    at += 2;
                }
                '$' if matches!(chars.get(at + 1), Some('{' | '(')) => {
                    let (open, close) = if chars[at + 1] == '{' {
                        ('{', '}')
                    } else {
                        ('(', ')')
                    };
                    let mut depth = 0;
                    while let Some(c) = chars.get(at) {
                        word.text.push(*c);
                        at += 1;
                        if *c == open {
                            depth += 1;
                        } else if *c == close {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                    }
                }
                // `<decision id>`: a placeholder may hold spaces.
                '<' if word.text.is_empty() && next_is_alpha => {
                    let close = chars[at + 1..]
                        .iter()
                        .take(80)
                        .position(|c| matches!(c, '>' | '<'))
                        .filter(|end| chars[at + 1 + end] == '>');
                    let len = close.map_or(1, |end| end + 2);
                    word.text.extend(&chars[at..at + len]);
                    at += len;
                }
                c => {
                    word.text.push(c);
                    at += 1;
                }
            }
        }
        words.push(word);
    }
    Ok(words)
}

/// A word that stands for a value the reader supplies, whatever the parser would make of it.
fn is_placeholder(word: &Word) -> bool {
    let text = &word.text;
    let angle = text.find('<').is_some_and(|open| {
        text[open..].contains('>') && text[open + 1..].starts_with(char::is_alphabetic)
    });
    let metavariable = text.len() <= 10
        && text.chars().any(|c| c.is_ascii_uppercase())
        && text
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_');
    text.contains('$')
        || text.contains("...")
        || text.contains('…')
        || angle
        || (!word.quoted && text.contains('|'))
        || metavariable
}

enum Expansion {
    Keep,
    Default(String),
    Drop,
}

/// `${LIMIT:-20}` is the 20 it falls back to; `${SESSION:+--session-id $SESSION}` is an optional
/// flag the example may or may not pass.
fn expansion(text: &str) -> Expansion {
    let Some(inner) = text.strip_prefix("${").and_then(|t| t.strip_suffix('}')) else {
        return Expansion::Keep;
    };
    match inner.split_once(":-") {
        Some((_, default)) => Expansion::Default(default.to_string()),
        None if inner.contains(":+") => Expansion::Drop,
        None => Expansion::Keep,
    }
}

/// The arguments after `hivemind`, with the shell and synopsis notation resolved, and whether the
/// command elided some of itself (`...`, `[--optional]`, `${FLAG:+--maybe}`).
fn arguments(words: Vec<Word>) -> (Vec<Arg>, bool) {
    let mut args = Vec::new();
    let mut elided = false;
    for mut word in words.into_iter().skip(1) {
        if word.text == "..." || word.text == "…" {
            elided = true;
            continue;
        }
        match expansion(&word.text) {
            Expansion::Keep => {}
            Expansion::Default(default) => word.text = default,
            Expansion::Drop => {
                elided = true;
                continue;
            }
        }
        let mut bracketed = false;
        if !word.quoted {
            if let Some(rest) = word.text.strip_prefix('[') {
                word.text = rest.to_string();
                bracketed = true;
            }
            if let Some(rest) = word.text.strip_suffix(']') {
                word.text = rest.to_string();
                bracketed = true;
            }
        }
        elided |= bracketed;
        if word.text.is_empty() {
            continue;
        }
        let placeholder = is_placeholder(&word) || (bracketed && !word.text.starts_with('-'));
        args.push(Arg {
            text: word.text,
            placeholder,
        });
    }
    (args, elided)
}

// ---------------------------------------------------------------------------------------------
// Asking the parser
// ---------------------------------------------------------------------------------------------

enum Next {
    Accept,
    Reject,
    Retry(usize, String),
}

fn context_string(err: &clap::Error, kind: ContextKind) -> Option<String> {
    match err.get(kind) {
        Some(ContextValue::String(text)) => Some(text.clone()),
        _ => None,
    }
}

/// What to make of a parse error: a gap a sketch is allowed (missing arguments), a placeholder the
/// parser cannot take as it stands (try another value), or a real mismatch with the CLI.
fn judge(err: &clap::Error, args: &[Arg], lenient: bool) -> Next {
    let placeholder_at = |value: &str| {
        args.iter()
            .position(|arg| arg.placeholder && arg.text == value)
    };
    match err.kind() {
        ErrorKind::DisplayHelp | ErrorKind::DisplayVersion => Next::Accept,
        ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
        | ErrorKind::MissingRequiredArgument
        | ErrorKind::MissingSubcommand
            if lenient =>
        {
            Next::Accept
        }
        ErrorKind::InvalidSubcommand => {
            let name = context_string(err, ContextKind::InvalidSubcommand).unwrap_or_default();
            if placeholder_at(&name).is_some() {
                Next::Accept
            } else {
                Next::Reject
            }
        }
        ErrorKind::UnknownArgument => {
            let given = context_string(err, ContextKind::InvalidArg).unwrap_or_default();
            let flag = given.split('=').next().unwrap_or_default();
            if placeholder_at(&given)
                .or_else(|| placeholder_at(flag))
                .is_some()
            {
                Next::Accept
            } else {
                Next::Reject
            }
        }
        ErrorKind::InvalidValue | ErrorKind::ValueValidation => {
            let value = context_string(err, ContextKind::InvalidValue).unwrap_or_default();
            if value.is_empty() {
                // A flag the sketch ends on, with no value after it.
                return if lenient { Next::Accept } else { Next::Reject };
            }
            let Some(at) = placeholder_at(&value) else {
                return Next::Reject;
            };
            let replacement = match err.get(ContextKind::ValidValue) {
                Some(ContextValue::Strings(valid)) => valid.first().cloned(),
                _ => None,
            };
            Next::Retry(at, replacement.unwrap_or_else(|| "1".to_string()))
        }
        _ => Next::Reject,
    }
}

/// Why the parser refuses this command, or `None` when it parses (or is not a command at all).
fn rejection(command: &str, fenced: bool) -> Option<String> {
    let words = match lex(command) {
        Ok(words) => words,
        Err(problem) => return Some(format!("the command has {problem}")),
    };
    let (mut args, elided) = arguments(words);
    // `hivemind 0.7.0` and `hivemind Up (healthy)` are output a block shows, not commands.
    let first = args.first()?;
    if first
        .text
        .starts_with(|c: char| c.is_ascii_uppercase() || c.is_ascii_digit())
    {
        return None;
    }
    let lenient = !fenced || elided;
    for _ in 0..MAX_PLACEHOLDER_RETRIES {
        let argv = std::iter::once("hivemind")
            .chain(args.iter().map(|arg| arg.text.as_str()))
            .map(str::to_string)
            .collect::<Vec<_>>();
        let Err(err) = Cli::try_parse_from(&argv) else {
            return None;
        };
        match judge(&err, &args, lenient) {
            Next::Accept => return None,
            Next::Reject => {
                let message = err.to_string();
                return Some(message.lines().next().unwrap_or_default().to_string());
            }
            Next::Retry(at, value) => {
                if args[at].text == value {
                    return None;
                }
                args[at].text = value;
                args[at].placeholder = false;
            }
        }
    }
    Some("too many placeholders to stand in for".to_string())
}

// ---------------------------------------------------------------------------------------------
// The tests
// ---------------------------------------------------------------------------------------------

#[test]
fn hivemind_commands_in_the_docs_parse() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut checked = 0;
    let mut broken = Vec::new();
    for path in markdown_files(root) {
        let file = path
            .strip_prefix(root)
            .expect("a repo file")
            .to_string_lossy()
            .replace('\\', "/");
        let text = fs::read_to_string(&path).expect("read a Markdown file");
        for example in examples_in(&file, &text) {
            checked += 1;
            if let Some(reason) = rejection(&example.text, example.fenced) {
                broken.push((example, reason));
            }
        }
    }
    // The extraction finding nothing would pass vacuously.
    assert!(
        checked > 150,
        "found only {checked} hivemind commands in the docs: did the extraction break?"
    );

    let mut used = BTreeSet::new();
    let mut unexplained = Vec::new();
    for (example, reason) in &broken {
        let allowed = ALLOWED
            .iter()
            .position(|a| a.file == example.file && example.text.starts_with(a.command));
        match allowed {
            Some(index) => {
                used.insert(index);
            }
            None => unexplained.push(format!(
                "{}:{}: {}\n    {}",
                example.file, example.line, example.text, reason
            )),
        }
    }
    let stale: Vec<String> = ALLOWED
        .iter()
        .enumerate()
        .filter(|(index, _)| !used.contains(index))
        .map(|(_, a)| format!("{}: {} ({})", a.file, a.command, a.why))
        .collect();

    assert!(
        unexplained.is_empty(),
        "{} of {checked} hivemind commands in the docs do not parse. Fix the example, or, if it \
         shows on purpose what the CLI does not do, add it to ALLOWED with the reason:\n{}",
        unexplained.len(),
        unexplained.join("\n")
    );
    assert!(
        stale.is_empty(),
        "these ALLOWED entries excuse nothing now (the example was fixed or is gone). Remove \
         them:\n{}",
        stale.join("\n")
    );
}

#[test]
fn readme_lists_every_mcp_tool() {
    let readme = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("README.md"))
        .expect("read README.md");
    let mut listed = BTreeSet::new();
    let mut in_table = false;
    for line in readme.lines() {
        if line.starts_with("| Tool |") {
            in_table = true;
        } else if in_table && !line.starts_with('|') {
            break;
        } else if in_table {
            let cell = line.split('|').nth(1).map(str::trim).unwrap_or_default();
            if let Some(name) = cell.strip_prefix('`').and_then(|c| c.strip_suffix('`')) {
                listed.insert(name.to_string());
            }
        }
    }
    assert!(
        !listed.is_empty(),
        "README.md has no `| Tool |` table of MCP tools"
    );

    let served: BTreeSet<String> = tool_definitions()
        .iter()
        .filter_map(|tool| tool["name"].as_str().map(str::to_string))
        .collect();
    let missing: Vec<&String> = served.difference(&listed).collect();
    let unknown: Vec<&String> = listed.difference(&served).collect();
    assert!(
        missing.is_empty() && unknown.is_empty(),
        "README's MCP table lists {} tools and the server has {}. Not in the table: {missing:?}. \
         In the table but not served: {unknown:?}",
        listed.len(),
        served.len()
    );
}

// The checks above are only as good as `rejection`: these pin what it accepts and refuses.

#[test]
fn a_whole_example_must_parse_but_a_sketch_may_leave_arguments_out() {
    assert!(rejection("hivemind --actor human:alice quickstart", true).is_none());
    assert!(rejection("hivemind query get_decision --id decision-001", true).is_none());
    // `--id` is required: a whole example without it is wrong, a prose mention of the verb is not.
    assert!(rejection("hivemind query get_decision", true).is_some());
    assert!(rejection("hivemind query get_decision", false).is_none());
    assert!(rejection("hivemind emit decision.proposed ...", true).is_none());
    assert!(rejection("hivemind query score_decision --id", false).is_none());
    assert!(rejection("hivemind query score_decision --id", true).is_some());
}

#[test]
fn a_retired_verb_or_flag_is_refused_even_in_a_sketch() {
    assert!(rejection("hivemind suggest document-candidates", true).is_some());
    assert!(rejection("hivemind why", false).is_some());
    assert!(rejection("hivemind query neighborhood --compact", false).is_some());
    assert!(rejection("hivemind query get_decision --no-such-flag 1", false).is_some());
    // `--actor` is a flag of `hivemind` itself, not of the subcommand it precedes.
    assert!(rejection("hivemind import documents ./notes --actor alice", true).is_some());
    assert!(rejection("hivemind dump --format svg", true).is_some());
}

#[test]
fn a_placeholder_stands_in_for_a_value_the_reader_supplies() {
    assert!(rejection("hivemind query get_decision --id <decision id>", true).is_none());
    assert!(rejection(
        "hivemind query recent_decisions --since 1h --limit <n>",
        true
    )
    .is_none());
    assert!(rejection("hivemind query recent_decisions --since 1h --limit N", true).is_none());
    assert!(rejection(
        "hivemind project link --from A --to B --kind part_of|depends_on",
        false
    )
    .is_none());
    assert!(rejection("hivemind query <tool> --id x", true).is_none());
    assert!(rejection("hivemind query get_decision --id $ID", true).is_none());
    // A placeholder does not excuse the flags around it.
    assert!(rejection("hivemind query get_decision --id <id> --no-such-flag", true).is_some());
}

#[test]
fn shell_syntax_around_a_command_is_read_the_way_a_shell_would() {
    assert!(rejection("hivemind dump --format dot > graph.dot", true).is_none());
    assert!(rejection(
        "hivemind classify-queue list --json --limit 1 | jq .pending_total",
        true
    )
    .is_none());
    assert!(rejection(
        "hivemind classify-queue list --json --limit ${LIMIT:-20} ${SESSION:+--session-id $SESSION}",
        true
    )
    .is_none());
    assert!(rejection(
        "hivemind query recall \"what is <x>?\" --limit 5 # why",
        true
    )
    .is_none());
    assert!(rejection("hivemind query recall 'oops", true).is_some());
    // Output a block shows, not a command.
    assert!(rejection("hivemind 0.7.0", true).is_none());
    assert!(rejection("hivemind Up (healthy) 127.0.0.1:8080", true).is_none());
}

#[test]
fn examples_are_found_in_shell_blocks_and_code_spans_only() {
    let doc = "\
Run `hivemind query get_decision --id x`, or a span that runs over
a line: `hivemind export --format
markdown --out ./out`. Not a command: `cargo run`, hivemind newer than 0.7.

```bash
$ hivemind --actor alice emit decision.proposed \\
    --title \"T\"
hivemind --version
```

```json
{ \"command\": \"hivemind nonsense\" }
hivemind nonsense
```
";
    let found: Vec<(usize, bool, String)> = examples_in("doc.md", doc)
        .into_iter()
        .map(|e| (e.line, e.fenced, e.text))
        .collect();
    assert_eq!(
        found,
        vec![
            (1, false, "hivemind query get_decision --id x".to_string()),
            (
                2,
                false,
                "hivemind export --format markdown --out ./out".to_string()
            ),
            (
                6,
                true,
                "hivemind --actor alice emit decision.proposed --title \"T\"".to_string()
            ),
            (8, true, "hivemind --version".to_string()),
        ]
    );
}
