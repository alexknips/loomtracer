//! Warnings on a capture that was recorded exactly as given (hivemind-ggoi).
//!
//! A warning is a shape check on the input, never a refusal: the decision is written either way,
//! with the option labels as the caller sent them. It carries no similarity and no ranking, so it
//! stays in the write layer. The verbs (CLI, MCP, REST) put the list in the reply as `warnings`,
//! and only when it is not empty.

use serde::Serialize;
use serde_json::{json, Value};

use crate::projector::slug_labels_with_words;

/// `CaptureWarning::kind` for an option label written as a slug.
pub const OPTION_LABEL_LOOKS_LIKE_A_SLUG: &str = "option_label_looks_like_a_slug";

/// One note on a recorded capture: what looked off, what it was, and what to write instead.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CaptureWarning {
    pub kind: &'static str,
    /// The option label as recorded.
    pub option_label: String,
    /// The words for it, as every reader will show it.
    pub suggestion: String,
    pub message: String,
}

/// A warning for each option label that is a slug: lowercase words joined by `-` or `_`
/// (`direct-cli`, `name-a-upheld`). A lone lowercase word (`sqlite`) is a fine label, and so is
/// ordinary hyphenated English (`e-mail`, `read-only`): the shape rule is in `option_labels`.
pub(super) fn option_label_warnings(option_labels: &[String]) -> Vec<CaptureWarning> {
    slug_labels_with_words(option_labels)
        .into_iter()
        .map(|(label, words)| CaptureWarning {
            kind: OPTION_LABEL_LOOKS_LIKE_A_SLUG,
            option_label: label.to_owned(),
            message: format!(
                "option label \"{label}\" looks like a slug; it is recorded as given and readers show \"{words}\". Write the words (\"{words}\") as the label"
            ),
            suggestion: words,
        })
        .collect()
}

/// Put `warnings` into a JSON reply, and nothing at all when there are none, so a clean capture
/// replies exactly as it did before warnings existed.
pub fn insert_warnings_into(warnings: &[CaptureWarning], reply: &mut Value) {
    if warnings.is_empty() {
        return;
    }
    if let Some(reply) = reply.as_object_mut() {
        reply.insert("warnings".to_owned(), json!(warnings));
    }
}
