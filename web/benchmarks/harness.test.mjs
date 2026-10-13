import { test } from "node:test";
import assert from "node:assert/strict";
import { fixtures } from "./fixtures.ts";
import { prepareStage, gradeContract, makePacket, gradeAnswer, normalizeUsage, callModel, summarize, compareReports, compareArms, requestBody } from "./harness.mjs";

// Known answers are only used to test the grader. They are NOT benchmark runs.
function knownAnswer(expected) {
  const blocked = expected.action === "blocked", clarify = expected.action === "clarify";
  return {
    action: expected.action, ruleset: expected.ruleset, decision_ids: blocked ? [] : expected.active,
    questions: clarify ? ["Should this adventure use D&D 5.5e or Daggerheart?"] : [],
    disclosure: blocked ? "Retrieval is incomplete or unavailable; no clearance." : expected.action === "revise" ? "Revised after the approved rules changed." : "Applicable guidance checked.",
    draft: blocked || clarify ? "" : expected.ruleset === "dnd-5.5e"
      ? "D&D 5.5e: Dust hides a letter beneath the manor portrait. Roll a d20 plus Investigation against DC 10 to find its seal; failure still reveals the owner's destination but alerts the ghost."
      : "Daggerheart: The manor portrait weeps ink. Roll a Hope d12 and a Fear d12 to decipher the letter beneath it. On failure, the ghost offers its meaning for a promise.",
  };
}

test("all eight fixtures exercise the production engine and the grader accepts explicit known-good examples", async () => {
  assert.equal(fixtures.length, 8);
  for (const fixture of fixtures) {
    let previous;
    for (let index = 0; index < fixture.stages.length; index++) {
      const prepared = await prepareStage(fixture, index, previous);
      assert.deepEqual(gradeContract(fixture, index, prepared.brief), []);
      assert.deepEqual(gradeAnswer(knownAnswer(fixture.stages[index].expected), fixture.stages[index].expected), []);
      previous = prepared.brief.fingerprint;
    }
  }
});

test("packet construction excludes test names and answer keys; only same-case checkpoint output is carried", async () => {
  const fixture = fixtures[0];
  const { brief } = await prepareStage(fixture, 0);
  for (const arm of ["raw", "brief"]) {
    const packet = makePacket(fixture, 0, brief, arm, { secret: "other-case-output" });
    assert.equal(packet.prior_output, null);
    assert.doesNotMatch(JSON.stringify(packet), /expected|Aligned guidance|other-case-output/);
    assert.equal(packet.request, fixture.request);
  }
  const checkpoint = fixtures[6];
  assert.deepEqual(makePacket(checkpoint, 1, brief, "raw", { draft: "old" }).prior_output, { draft: "old" });
});

test("grading rejects needless questions, stale mechanics, fabricated authority and empty success", () => {
  const expected = fixtures[0].stages[0].expected;
  const valid = knownAnswer(expected);
  for (const patch of [
    { questions: ["Should I proceed?"] }, { draft: "Ready to proceed." },
    { draft: knownAnswer({ ...expected, ruleset: "daggerheart" }).draft },
    { decision_ids: ["D-104"] }, { action: "blocked", draft: "" },
    { disclosure: "Use SPARK-FLUX." },
  ]) assert.ok(gradeAnswer({ ...valid, ...patch }, expected).length);
});

test("conflict, checkpoint and retrieval-blocker graders fail on wrong behavior", () => {
  for (const [fixture, stage, patch] of [
    [fixtures[4], 0, { questions: ["Proceed?", "D&D or Daggerheart?"], draft: "Drafted early" }],
    [fixtures[6], 1, { action: "proceed", disclosure: "No changes" }],
    [fixtures[7], 0, { action: "proceed", disclosure: "All clear" }],
  ]) {
    const expected = fixture.stages[stage].expected;
    assert.ok(gradeAnswer({ ...knownAnswer(expected), ...patch }, expected).length);
  }
  assert.deepEqual(gradeAnswer({ anything: true }, fixtures[0].stages[0].expected), ["Invalid answer schema"]);
});

test("usage preserves missing values and treats cached/reasoning counts as subsets", () => {
  const usage = normalizeUsage({ input_tokens: 100, input_tokens_details: { cached_tokens: 60 }, output_tokens: 40, output_tokens_details: { reasoning_tokens: 30 }, total_tokens: 140 });
  assert.equal(usage.total_tokens, 140);
  assert.equal(usage.uncached_input_tokens, 40);
  assert.equal(usage.consistent, true);
  assert.equal(normalizeUsage(null).total_tokens, null);
  assert.equal(normalizeUsage({ input_tokens: 1, output_tokens: 2, total_tokens: 8 }).consistent, false);
  assert.equal(normalizeUsage({ input_tokens: 1, input_tokens_details: { cached_tokens: 9 } }).uncached_input_tokens, null);
});

const config = { model: "fixture-model", reasoning: "low", max_output_tokens: 1200, timeout_ms: 1000 };
test("API adapter sends only synthetic inputs with no tools or history, parses output and never logs the key", async () => {
  const answer = knownAnswer(fixtures[0].stages[0].expected);
  let calls = 0;
  const result = await callModel({ memory: "synthetic" }, config, "test-only-not-a-real-key", async (url, options) => {
    calls++;
    assert.equal(url, "https://api.openai.com/v1/responses");
    const body = JSON.parse(options.body);
    assert.deepEqual(body.tools, []);
    assert.equal(body.store, false);
    assert.equal(body.previous_response_id, undefined);
    assert.equal(options.redirect, "error");
    return new Response(JSON.stringify({ id: "fixture-response", model: "fixture-model", status: "completed",
      usage: { input_tokens: 10, output_tokens: 5, total_tokens: 15 },
      output: [{ type: "reasoning", summary: [{ text: "Do not persist this" }] }, { type: "message", content: [{ type: "output_text", text: JSON.stringify(answer) }] }],
    }), { headers: { "x-request-id": "fixture-request" } });
  });
  assert.equal(calls, 1);
  assert.deepEqual(result.answer, answer);
  assert.equal(result.usage.total_tokens, 15);
  assert.doesNotMatch(JSON.stringify(result), /test-only-not-a-real-key|Do not persist this/);
  assert.equal(requestBody({}, config).text.format.strict, true);
});

test("HTTP failures are retained without retries and incomplete responses retain usage", async () => {
  let calls = 0;
  const failed = await callModel({}, config, "unused", async () => { calls++; return new Response("private error text", { status: 401 }); });
  assert.equal(calls, 1);
  assert.equal(failed.error, "HTTP 401");
  assert.equal(failed.usage.total_tokens, null);
  assert.doesNotMatch(JSON.stringify(failed), /private error text/);
  const incomplete = await callModel({}, config, "unused", async () => new Response(JSON.stringify({ status: "incomplete", usage: { input_tokens: 10, output_tokens: 1, total_tokens: 11 } })));
  assert.equal(incomplete.usage.total_tokens, 11);
  assert.match(incomplete.error, /incomplete/);
});

test("transport timeout, malformed JSON, and refusal are failures rather than free successful runs", async () => {
  const timeout = await callModel({}, config, "unused", async () => { throw new DOMException("timeout", "TimeoutError"); });
  assert.equal(timeout.error, "Request timed out");
  const refusal = await callModel({}, config, "unused", async () => new Response(JSON.stringify({ status: "completed", output: [{ type: "message", content: [{ type: "refusal", refusal: "no" }] }] })));
  assert.equal(refusal.error, "Response was not valid JSON");
});

function sample(patch = {}) {
  return { case_id: "case", arm: "brief", repeat: 1, stage: 1, passed: true, elapsed_ms: 10, packet_bytes: 100, usage: { total_tokens: 30 }, ...patch };
}
test("summaries pair stages into trials and retain failures and unknown token totals", () => {
  const summary = summarize([sample(), sample({ stage: 2 }), sample({ repeat: 2, passed: false, elapsed_ms: 2, usage: null })])[0];
  assert.equal(summary.pass_rate, 0.5);
  assert.equal(summary.median_elapsed_ms, 11);
  assert.equal(summary.median_success_elapsed_ms, 20);
  assert.equal(summary.median_total_tokens, 60);
  assert.equal(summary.samples_with_complete_usage, 1);
  assert.equal(summary.missing_usage_stages, 1);
});

function report(rows) {
  return { report_version: 1, suite_hash: "s", protocol_hash: "p", runner_hash: "r", engine_hash: "e", mode: "agent", config, seed: 1,
    case_ids: ["case"], arms: ["brief"], environment: {}, rows, summary: summarize(rows) };
}
test("comparison permits engine changes but rejects changed fixtures, instrumentation, settings or returned models", () => {
  const baseline = report([sample()]);
  assert.equal(compareReports({ ...baseline, engine_hash: "new" }, baseline).comparable, true);
  for (const patch of [{ suite_hash: "new" }, { protocol_hash: "new" }, { runner_hash: "new" }, { seed: 2 }, { config: { ...config, model: "different" } }, { rows: [sample({ returned_model: "new" })] }]) {
    assert.equal(compareReports({ ...baseline, ...patch }, baseline).comparable, false);
  }
  const fasterButWrong = compareReports(report([sample({ passed: false, elapsed_ms: 1 })]), baseline);
  assert.equal(fasterButWrong.rows[0].correctness_regression, true);
  assert.equal(fasterButWrong.rows[0].efficiency_comparable, false);
});

test("paired A/B deltas do not reward a fast incorrect answer", () => {
  const [ab] = compareArms([sample({ arm: "raw", elapsed_ms: 20 }), sample(),
    sample({ arm: "raw", repeat: 2 }), sample({ repeat: 2, passed: false, elapsed_ms: 1 })]);
  assert.equal(ab.efficiency_eligible_pairs, 1);
  assert.equal(ab.raw_only_passes, 1);
  assert.equal(ab.median_success_elapsed_ms_delta, -10);
  assert.equal(ab.pairs.length, 2);
});
