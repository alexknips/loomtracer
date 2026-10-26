use super::slug_labels_with_words;

/// The labels that get a warning, each with its suggestion, for one capture's option labels.
fn warned(labels: &[&str]) -> Vec<(String, String)> {
    let labels: Vec<String> = labels.iter().map(|label| (*label).to_owned()).collect();
    slug_labels_with_words(&labels)
        .into_iter()
        .map(|(label, words)| (label.to_owned(), words))
        .collect()
}

fn pair(label: &str, words: &str) -> (String, String) {
    (label.to_owned(), words.to_owned())
}

#[test]
fn ordinary_hyphenated_words_get_no_warning() {
    let ordinary = [
        "e-mail",
        "opt-in",
        "opt-out",
        "read-only",
        "follow-up",
        "real-time",
        "built-in",
        "self-hosted",
        "Read-only",
        "Read-only replica",
        "read-only replica",
        "Self-hosted Postgres",
    ];
    for label in ordinary {
        assert_eq!(warned(&[label]), Vec::new(), "{label} is an ordinary label");
    }
    // And together, in one capture.
    assert_eq!(warned(&ordinary), Vec::new());
}

#[test]
fn a_label_among_others_is_judged_on_its_own() {
    assert_eq!(
        warned(&["read-only", "pick-a-postgres", "Hosted API", "e-mail"]),
        vec![pair("pick-a-postgres", "Pick a postgres")]
    );
}

#[test]
fn slugs_still_get_a_warning_with_the_words() {
    assert_eq!(
        warned(&["pick-a-postgres", "pick-b-sqlite", "pick-c-cockroach"]),
        vec![
            pair("pick-a-postgres", "Postgres"),
            pair("pick-b-sqlite", "Sqlite"),
            pair("pick-c-cockroach", "Cockroach"),
        ]
    );
    // Alone there is no lettered set to read the letter from, so it stays in the words.
    assert_eq!(
        warned(&["pick-a-postgres"]),
        vec![pair("pick-a-postgres", "Pick a postgres")]
    );
    // An underscore is never ordinary English, even between two parts.
    assert_eq!(
        warned(&["use_postgres", "snake_case_label"]),
        vec![
            pair("use_postgres", "Use postgres"),
            pair("snake_case_label", "Snake case label"),
        ]
    );
    // Three or more parts, joined by either mark.
    assert_eq!(
        warned(&["rest-api-over-http", "use-local-ledger", "use_local_ledger"]),
        vec![
            pair("rest-api-over-http", "Rest API over HTTP"),
            pair("use-local-ledger", "Use local ledger"),
            pair("use_local_ledger", "Use local ledger"),
        ]
    );
}

#[test]
fn a_lowercase_acronym_marks_a_two_part_label_as_a_slug() {
    // Prose writes `CLI` and `MCP`; only a slug has them in lowercase.
    assert_eq!(
        warned(&["direct-cli", "mcp-server"]),
        vec![
            pair("direct-cli", "Direct CLI"),
            pair("mcp-server", "MCP server"),
        ]
    );
    // Whole parts only: `cliff` merely starts with the letters of `cli`.
    assert_eq!(warned(&["cliff-edge", "guided-tour"]), Vec::new());
}

#[test]
fn a_two_part_hyphenated_label_without_a_tell_gets_no_warning() {
    // No shape tells `direct-store` from `read-only`, so neither is flagged: a missed slug
    // costs less than a warning on a good label.
    assert_eq!(warned(&["direct-store", "local-ledger"]), Vec::new());
}

#[test]
fn a_lone_lowercase_word_or_a_phrase_is_not_a_slug() {
    assert_eq!(
        warned(&["sqlite", "Direct CLI", "use local ledger"]),
        Vec::new()
    );
}

#[test]
fn no_suggestion_holds_a_stray_letter() {
    // Three parts or an underscore make these slug-shaped, but the words would hold the half
    // of a word the slug split (`E mail`), which is worse than the label: no warning.
    assert_eq!(
        warned(&[
            "e-mail-server",
            "k-means-clustering",
            "use_x_ray",
            "t-shirt-size"
        ]),
        Vec::new()
    );
    // `a` and `I` are words of one letter.
    assert_eq!(
        warned(&["pick-a-postgres", "where-i-stand"]),
        vec![
            pair("pick-a-postgres", "Pick a postgres"),
            pair("where-i-stand", "Where i stand"),
        ]
    );
    // A digit is not a letter.
    assert_eq!(
        warned(&["option-3-first"]),
        vec![pair("option-3-first", "Option 3 first")]
    );
}

#[test]
fn every_suggestion_of_a_warned_label_is_free_of_stray_letters() {
    let labels = [
        "pick-a-postgres",
        "pick-b-sqlite",
        "q1-a-drop-the-line",
        "e-mail-server",
        "x-ray-tool",
        "use-x-ray-tool",
        "opt-in",
        "direct-cli",
        "use_postgres",
    ];
    for (label, words) in warned(&labels) {
        assert!(
            !super::has_stray_letter(&words),
            "{label} would suggest {words}"
        );
    }
}
