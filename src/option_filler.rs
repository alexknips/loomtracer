//! The text a capture surface fills in when the author gave an option no description, and the
//! one recogniser that reads it back (hivemind-ox14j).
//!
//! Each surface writes its filler through the function named for it here, and the Alternatives
//! floor ([`crate::quality_profile`]) decides whether a description is only filler through
//! [`is_generated`], which reads the same templates. A surface that rewords its filler changes
//! this file and the floor follows; there is no second copy of the text to forget. The rule is
//! documented in `docs/DECISION_SCORING.md`.

const MCP_PREFIX: &str = "Option generated from MCP value '";
const CLI_PREFIX: &str = "Option generated from CLI value '";
const SUPERSEDE_PREFIX: &str = "Option generated from supersede value '";
const HTTP_PREFIX: &str = "Option '";
const SLACK_PREFIX: &str = "Slack option '";
const SLACK_INFIX: &str = "' captured from ";
const IMPORT_PREFIX: &str = "Option imported from document block ";

/// The surfaces whose filler is `<prefix><label>'`.
const QUOTED_PREFIXES: [&str; 4] = [MCP_PREFIX, CLI_PREFIX, SUPERSEDE_PREFIX, HTTP_PREFIX];

fn quoted(prefix: &str, label: &str) -> String {
    format!("{prefix}{label}'")
}

/// MCP `capture_decision`.
pub(crate) fn mcp(label: &str) -> String {
    quoted(MCP_PREFIX, label)
}

/// CLI `emit decision.proposed` and `decision.capture`.
pub(crate) fn cli(label: &str) -> String {
    quoted(CLI_PREFIX, label)
}

/// `supersede`.
pub(crate) fn supersede(label: &str) -> String {
    quoted(SUPERSEDE_PREFIX, label)
}

/// HTTP capture.
pub(crate) fn http(label: &str) -> String {
    quoted(HTTP_PREFIX, label)
}

/// Slack ingest. `source_ref` is the thread the option came from.
pub(crate) fn slack(label: &str, source_ref: &str) -> String {
    format!("{SLACK_PREFIX}{label}{SLACK_INFIX}{source_ref}")
}

/// Document import. `block_id` is the block the option was imported from.
pub(crate) fn import(block_id: &str) -> String {
    format!("{IMPORT_PREFIX}{block_id}")
}

/// Whether `description` is only the option's own label or text a capture surface generated for
/// it. Such text carries nothing the author wrote, so it does not count as a description.
///
/// The quoted surfaces match exactly, so an author who kept the generated text and added a reason
/// has written a description of their own. Slack and document import end in a free-form tail (the
/// thread, the block id) that cannot be told from words added after it, so for those two anything
/// after the fixed text still reads as generated.
pub(crate) fn is_generated(label: &str, description: &str) -> bool {
    if description.eq_ignore_ascii_case(label) || description.starts_with(IMPORT_PREFIX) {
        return true;
    }
    if QUOTED_PREFIXES.iter().any(|prefix| {
        description
            .strip_prefix(prefix)
            .and_then(|rest| rest.strip_prefix(label))
            == Some("'")
    }) {
        return true;
    }
    description
        .strip_prefix(SLACK_PREFIX)
        .and_then(|rest| rest.strip_prefix(label))
        .is_some_and(|rest| rest.starts_with(SLACK_INFIX))
}

#[cfg(test)]
mod tests;
