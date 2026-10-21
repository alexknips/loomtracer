# Reasoning continuity benchmark v5

A development benchmark for testing whether agents preserve earlier reasoning and use it correctly after a handoff. The intended comparison is a stock model with maintained notes, a stock model with conversation history, and the same model with structured memory. This package currently runs the **maintained-notes calibration condition only**; it does not yet integrate with this repository's native HiveMind implementation.

**Ready now:** provider-free tests, all 27 synthetic scenarios, deterministic simulated tasks, immutable evidence snapshots, strict scoring, token accounting, and a gated three-case live calibration runner. **Not yet established:** calibrated difficulty, an audited leaderboard, or a measured advantage over either stock baseline. Offline success verifies the harness, not model performance.

## Start with offline testing

Requires Node.js 22 or later and npm. From the repository root:

```sh
cd benchmarks/functionality-v5
npm ci
npm test
npm run validate
```

`npm ci` installs the pinned `js-tiktoken@1.0.21` dependency using the included lockfile. Tests and validation make **zero model/provider calls** and need no subscription credentials. Validation creates a new, ignored `.artifacts/offline-*` directory containing test output, the generated corpus, source hashes, a frozen implementation, and an independent-review template. Existing output directories are refused.

For an explicit location:

```sh
npm run validate -- --output /absolute/new/offline-directory
```

Do not confuse the tokenizer's context-sizing proxy with measured usage. Expenditure comes only from saved CLI usage records.

## What is tested

| Component | Test | Strict result |
| --- | --- | --- |
| H — reasoning preservation | Reconstruct premises, relationships, attribution, and changes after intervening work and handoffs. | Each complete, unambiguous, source-supported unit passes or fails. Citation syntax is reported separately. |
| Q — question handling | Use known answers, ask for genuinely missing active inputs, and avoid irrelevant questions. | Both necessary and unnecessary-question behavior count. |
| W — work reuse | Reuse applicable receipts, repeat stale work, and finish the currently authorized task. | Valid reuse and necessary fresh work are both required. |
| D — duplicate recognition | Distinguish the same underlying problem, a distinct problem with shared details, and unresolved identity before any substantive reuse, execution, or observation. | A correct identity explanation delivered too late fails recognition. |

Ordinary action tasks and explicitly prompted recognition tasks run independently from the same checkpoint. Their scores stay separate. H has its own independent session. No probe answer or grade feeds back into maintained memory.

There are **27 cases: three workflow families × three difficulty levels × three identity classes**. The first live screen selects one demanding case per family:

| Screen case | Reasoning to retain | Work to perform |
| --- | --- | --- |
| Service timeout investigation — same incident | Joint queue/DNS evidence, who recommended and authorized a mitigation, staging-only limits, and a later batch-size exception. | Recognize a paraphrased ticket; reuse unchanged platform checks; obtain missing inputs and run a fresh larger-batch trial. |
| Experimental protocol — distinct study | Sensitivity and staffing jointly support a limited method choice; a later request changes population and batch conditions. | Recognize a distinct study; reuse instrument checks; obtain buffer, volume, and operator inputs; perform fresh batch analysis. |
| Release coordination — unresolved deliverable | Two reasons constrain the release window; approval and authorship have different roles; an attribution correction is not a reversal. | Recognize unresolved identity; reuse common-asset checks; prepare a changed locale where authorized; stop distribution at unresolved authority. |

The tasks use synthetic tools and receipts, not real deployments, clinical experiments, or releases. See [PROTOCOL.md](PROTOCOL.md) for scoring, controls, difficulty, and accounting. The executable corpus in `lib/benchmarks/functionality-v5/corpus.mjs` is the source of truth; validation generates its inspectable JSON form.

## Live calibration is a separate, explicit step

```sh
npm run screen -- --help
npm run validate -- --help
```

Before live preparation, an independent reviewer must inspect the authored sources and complete the generated `authoring-review.template.json`. Validate again into a **new directory**, supplying that review with `--review`. Hash checks bind approval to the exact reviewed sources. Passing unit tests does not supply semantic approval.

The operator must separately authorize one fresh run in a local JSON file with `operator`, `approved: true`, `purpose`, `approvedAt` (ISO timestamp), and `budgetTokens`. The maximum accepted ceiling is **4,250,000 CLI-reported tokens**, covering acquisition, maintenance, task execution, controls, grading, and canaries. Choose the actual budget deliberately; the ceiling is not permission to spend it, and no earlier project's approval carries over.

```sh
npm run screen -- --output /absolute/new/run-directory \
  --offline-root /absolute/reviewed/offline-directory \
  --authorization /absolute/operator-authorization.json
```

This command prepares immutable artifacts without provider calls. A live invocation adds `--execute`; `--execute --canaries-only` limits a fresh diagnostic to its isolation canary. Read the help before starting. The public CLI accepts fresh output directories only: historical private-run continuation and failed-call retry exceptions are intentionally excluded.

The live runner is deliberately pinned to macOS, ChatGPT subscription authentication, Codex CLI `0.163.0-alpha.10.2` at the bundled ChatGPT.app path, requested model `gpt-6.1-sol`, medium reasoning, and standard processing. These are experimental pins, not a promise that the installation or model is available on another machine. An unsupported installation fails closed. Porting the runner requires a new recorded protocol and fresh isolation verification; it must not silently change models or switch to API billing.

Every live screen begins with fresh isolation canaries and grader qualification. Evaluated sessions use ephemeral empty workspaces, with unrelated tools, integrations, repository instructions, and automatic memories disabled. Historical access goes through the bounded controller. Dispatch is paced at least 60 seconds apart; each invocation has a 120-second timeout. The controller owns its `caffeinate` process and requests its termination on exit. Failed calls and unknown usage stop subsequent dispatch; there are no automatic retries or API fallbacks. A token threshold cannot cap an invocation already in flight.

## Evidence and current limits

The export is additive. It contains no prior private transcripts, credentials, results, or token ledgers, and does not modify v1–v4 or earlier v5 runs. [CHANGES.md](CHANGES.md) describes its versioned repairs. [Source provenance](provenance/source-snapshot.json) records the upstream files used for the export; each validation also hashes the actual exported implementation.

Earlier calibration exposed a missing producer-role premise, a duplicate question penalty, and a late-recognition display issue. Those are repaired here with regression tests. Earlier ratings are not republished as corrected measurements of this new corpus.

This remains a small development screen. Human semantic audit is required; uncertain judgments remain pending. H previously showed ceiling effects, and ordinary versus prompted D showed extreme separation, so the screen does not yet demonstrate useful discrimination across all four components. The native-memory and conversation-history conditions, held-out evaluation, repeated runs, and a long-history expansion need their own integration and validation before comparative claims.

The controller knows the hidden criteria; evaluated agents must not receive them. Publishing the synthetic corpus makes offline inspection possible, but running an agent inside this repository with unrestricted access would invalidate the evaluation.

This is a standalone Node benchmark contribution. It changes no Rust application, database, build, or CI behavior. Repository licensing and contribution policies apply.
