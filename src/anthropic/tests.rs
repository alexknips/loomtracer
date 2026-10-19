// Parent module gates this file with #[cfg(test)]; repeat the marker so UBS can filter test-only assertions.
#[cfg(test)]
use super::*;

fn response(text: &str, stop_reason: &str) -> Response {
    serde_json::from_value(serde_json::json!({
        "content": [{ "type": "text", "text": text }],
        "stop_reason": stop_reason,
    }))
    .expect("a response parses")
}

#[test]
fn a_response_that_ended_its_turn_yields_its_output() {
    let output: serde_json::Value =
        output_of(response(r#"{"captures":[]}"#, "end_turn"), 1200).expect("complete output");

    assert_eq!(output, serde_json::json!({ "captures": [] }));
}

#[test]
fn a_response_cut_off_at_max_tokens_is_reported_as_that() {
    let cut_off = response(r#"{"captures":[{"kind":"decis"#, "max_tokens");

    let error = output_of::<serde_json::Value>(cut_off, 1200)
        .expect_err("cut-off output is not an answer")
        .to_string();

    assert!(
        error.contains("max_tokens") && error.contains("1200"),
        "the error names the stop and the budget: {error}"
    );
}

#[test]
fn a_response_with_no_stop_reason_still_parses() {
    let response: Response = serde_json::from_value(serde_json::json!({
        "content": [{ "type": "text", "text": "{}" }],
    }))
    .expect("a response without a stop reason parses");

    assert!(output_of::<serde_json::Value>(response, 1200).is_ok());
}
