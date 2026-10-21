// Seeds a scratch HiveMind ledger with a small, fictional pricing study: two projects (one part of
// the other), every decision status, a cross-project edge and a refuted assumption. Used by the
// e2e scratch server and to capture lib/hivemind/fixtures/graph.json. Needs `hivemind` on PATH
// (or HIVEMIND_BIN).
import { execFileSync } from "node:child_process";
import { writeFileSync } from "node:fs";
import { join } from "node:path";

export const SEED_TOKEN = "e2e-scratch-token";

export function seedLedger(dir, bin = process.env.HIVEMIND_BIN || "hivemind") {
  const run = (actor, ...args) => execFileSync(bin, ["--hivemind-dir", dir, "--actor", actor, ...args], { encoding: "utf8", env: { ...process.env, HIVEMIND_API_URL: "", HIVEMIND_DATABASE_URL: "" } });
  const json = (actor, ...args) => JSON.parse(run(actor, ...args, "--json").trim().split("\n").at(-1));
  const captureReply = (actor, project, fields) => json(actor, "emit", "decision.capture", "--project", project, ...Object.entries(fields).flatMap(([flag, value]) => value === true ? [`--${flag}`] : [`--${flag}`, String(value)]));
  const capture = (...args) => captureReply(...args).value;

  json("human:alice", "project", "register", "pricing", "--display-name", "Pricing study", "--purpose", "Which pricing model do we test next");
  json("human:alice", "project", "register", "pricing-experiments", "--display-name", "Pricing experiments", "--purpose", "Experiments under the pricing study");
  json("human:alice", "project", "link", "--from", "pricing-experiments", "--to", "pricing", "--kind", "part_of");
  json("human:alice", "project", "declare-topic", "pricing", "pricing", "seats", "team-plan", "invite");
  json("human:alice", "project", "declare-topic", "pricing-experiments", "pricing", "experiment");

  const perSeat = capture("human:alice", "pricing", {
    title: "Price per seat", rationale: "A fixed seat count is easy for teams to budget for and easy for us to explain",
    "topic-keys": "pricing,seats", options: "Per seat,Usage based", chose: "Per seat",
    "rests-on-assumption": "Teams prefer a bill they can predict over one that follows activity",
    binds: "The first pricing experiment", "cheap-to-undo": true,
  });
  const teamPlan = capture("agent:claude:analyst", "pricing", {
    title: "Test a team plan with included usage",
    rationale: "Per-seat charges made occasional collaborators hard to invite, usage billing made bills hard to predict; an included allowance combines both",
    "topic-keys": "pricing,team-plan", options: "Team plan with allowance,Pure per seat,Pure usage", chose: "Team plan with allowance",
    "rests-on-evidence": "Seat study: occasional collaborators were invited less often when each seat cost extra", "evidence-source": "experiment-01",
    binds: "The next pricing experiment", "undo-cost": "One experiment cycle",
  });
  json("agent:claude:analyst", "emit", "decision.superseded", "--old", perSeat, "--new", teamPlan);

  // Proposed, never accepted: it waits for a person. Resting on a decision of the parent project makes a cross-project edge.
  const experiment = captureReply("agent:claude:analyst", "pricing-experiments", {
    title: "Run the allowance experiment on the invite flow",
    rationale: "The invite flow is where seat cost showed up, so it is where an included allowance would show up too",
    "topic-keys": "pricing,experiment", options: "Invite flow,Billing page", "still-proposed": true,
    "rests-on-decision": teamPlan, "rests-on-assumption": "Invite behaviour is a fair proxy for willingness to collaborate",
    binds: "The experiment design", "cheap-to-undo": true,
  });

  // Contested: bob accepted, carol rejected.
  const invite = capture("human:bob", "pricing", {
    title: "Show the price before the invite is sent",
    rationale: "People hesitate when they learn the cost after clicking invite, so the price comes first",
    "topic-keys": "pricing,invite", options: "Price first,Price after", chose: "Price first",
    "rests-on-assumption": "Surprise is what makes people hesitate", binds: "The invite dialog copy", "cheap-to-undo": true,
  });
  run("human:carol", "disagree", "--decision", invite, "--reason", "Showing the price first may suppress invites exactly as the seat study warned");

  // A question nobody has answered yet.
  json("human:alice", "ask", "Should the team plan include a free tier?");

  // Evidence that refutes the assumption the experiment decision rests on.
  const proxy = experiment.rests_on.find(premise => premise.kind === "assumption").id;
  const evidence = json("human:alice", "emit", "evidence.recorded", "--content", "Interview notes: teams also invite people for reasons unrelated to price").value;
  json("human:alice", "emit", "relation.added", "--kind", "refutes", "--from", evidence, "--to", proxy);
  return { perSeat, teamPlan, experiment: experiment.value, invite, evidence, proxy };
}

// A classifier draft: a decision read out of a session, whose id nobody chose (`capture:<event>:<index>`, colons in it).
// Most decisions in a real ledger are drafts, so the e2e server holds two: one that chose among its options, one that
// chose words it never weighed. Kept apart from seedLedger: they land in the submitter's personal project, and
// lib/hivemind/fixtures is captured from the pricing study alone.
export function seedDraft(dir, bin = process.env.HIVEMIND_BIN || "hivemind") {
  const captures = join(dir, "draft-captures.json");
  writeFileSync(captures, JSON.stringify([{
    kind: "decision", title: "Chose a weekly release train over releasing on demand",
    rationale: "A fixed weekly train gives support a date to plan around, and a late fix waits at most six days",
    topic_keys: ["releases"], evidence_ids: [], options: ["Weekly release train", "Release on demand"],
    chosen_option: "Weekly release train", extraction_confidence: 0.9,
  }, {
    // A choice it never weighed as an option: the draft names "Rebase in a scratch worktree" and lists two others.
    kind: "decision", title: "Rebase the hotfix branch onto main before the fast-forward merge",
    rationale: "Main moved while the branch was open and the two sides share no files, so a rebase keeps the history straight",
    topic_keys: ["releases"], evidence_ids: [], options: ["Merge commit", "Squash merge"],
    chosen_option: "Rebase in a scratch worktree", extraction_confidence: 0.9,
  }]));
  execFileSync(bin, ["--hivemind-dir", dir, "--actor", "agent:hivemind:classifier", "emit", "ingest.batch_classified", "--captures", captures, "--source", "agent", "--json"],
    { encoding: "utf8", env: { ...process.env, HIVEMIND_API_URL: "", HIVEMIND_DATABASE_URL: "" } });
}
