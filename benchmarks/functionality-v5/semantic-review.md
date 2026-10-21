# Portable v5 semantic authoring review

Status: passed with explicit scope limitations. Bound to the exact SHA-256 bytes of all 14 required source files; `authoring-review.json` passes `validateAuthoringReview`. This is prospective agent-assisted authoring review, not a retrospective human audit of prior runs. No evaluated agents or provider calls were used.

## What was reviewed

All 27 generated cases, 432 H requirement instances, all Q/W obligations, each same/distinct/unresolved identity source, four-episode source chronology, current public action contracts, ordinary/prompted distinction, oracle and evidence-withheld controls, source/Markdown/native delivery boundaries, grading packets and scoring. A successful offline originating witness binds actual synthetic receipts only; it is never passed to an evaluated agent as a curated handoff.

The detailed source and criterion coverage is in semantic-review-evidence.json. All 27 primary and withheld witness paths have legal terminal outcomes, alternative independent work orders remain allowed, and primary plans leave at least four broker operations beyond one counted lookup. Focused tests: 83 passed, zero failed/skipped. Corpus/world validation: 786 checks, passed. These checks prove harness properties and authoring consistency, not model performance.

## Findings and prospective corrections

- **S1: Original release key required producer=Priya without an explicit source assignment.** Portable corpus requires an explicit unqualified originating role declaration by Priya; all nine release cases and both controls retain it. Missing, negated, quoted, conflicting, wrong-actor or later substitutes fail regression validation. References: corpus.mjs:101; validation.mjs:17.
- **S2: One unnecessary portable check recomputation failed both its reuse unit and the work guard.** Artifact-specific repeat fails that unit once. Guard excludes that duplicate penalty while still failing unlisted repeats, inactive work and repeated successful current operations. References: world.mjs:deterministicChecks; corpus.mjs:w-no-extraneous-work.
- **S3: Only the last recognize record determined D timing, so a late restatement could erase a correct early decision.** All timing candidates remain visible; deterministic late requires no timely candidate. Semantic review identifies earliest complete supported decision. Early ambiguity plus late correctness still fails; a supported early decision plus late repetition remains eligible. References: world.mjs:identityDecisions; world.mjs:deterministicChecks; grading.mjs:gradePrompt.
- **S4: The broker forced blocked status for lower-level release subtasks even though every requested operation was identity-independent.** Block only if a requested operation depends on missing identity/evidence. A completed narrow release validation task can finish while broader release remains unapproved. Demanding unresolved release still blocks manifest assembly. References: world.mjs:blockedRequestedWork; validation.mjs:withheld plan; corpus.mjs:publicRules.
- **S5: Review packet omitted the exact public action contract, operation budget and actual delivered passage context.** Packet carries exact public world contract and task, consumed operation count, and whitelisted actually delivered source/Markdown/native passages with first-delivery invocation. No private archive, native workspace or arm identity is copied. References: grading.mjs:basePacket; grading.mjs:deliveredPassages.
- **S6: Current-configuration dependency had eligible support in src-0012 but omitted that exact occurrence from its frozen source pointer list.** Add src-0012 to that H criterion. Semantic obligation and denominator remain unchanged. References: corpus.mjs:h-relationships-3 incident.

- **S7: A literal historical artifact ID in the native recall query echo could be mistaken for retained evidence even when zero results returned.** Artifact eligibility now extracts text only from actual returned retained native decision content, allowed evidence/option/hypothesis snippets or neighborhoods. Query echoes, sidecar keys, ranking diagnostics and generated digests are excluded; whole-identifier boundaries apply. Real-native negative/positive regression confirms a query echo cannot unlock reuse while returned retained content can. References: native.mjs:191; workflow.mjs:15. The owner reported 24 focused offline tests passing; this reviewer inspected the extraction allowlist and actual-result gate.
- **S8: Mandatory audit selection could omit uncertain identity, completion or withheld-control judgments without a global uncertainty string.** All judgment types now require audit when uncertain/disputed or materially contradictory. An eight-packet same-stratum regression proves those packets are retained beyond the 25% minimum. References: grading.mjs:humanAuditSelection; grading.test.mjs.

- **S9: A global semantic uncertainty could leave per-case numeric scores or otherwise-pass D/chain/task labels looking definitive after aggregate displays became pending.** Arm, band and case ratings become provisional/pending; otherwise-pass D, complete-chain and task statuses remain pending. Original unit judgments, counts and established failures are preserved. References: report.mjs:93. This reviewer ran all 13 report tests successfully.

The portable corpus is 1.0.3, world contract 1.0.3, strict rubric 1.0.2 and blinded grading 1.0.1. The old rubric hash remains recorded in the test comment and prior snapshots. No saved workflow, source, grade or token ledger was reinterpreted.

## Memory dependence and limits

H tests source-backed historical assertions, joint reasons, scopes and changes. Actual observations are scored only if acquisition established them; otherwise explicit uncertainty is correct. W requires visible retained references to actual earlier artifacts and checks observed reuse/fresh execution. D uses registry identity rather than shared numbers, requires recognition before work, and has reduced-evidence controls. Ordinary D measures voluntary outward behavior; the same capability explicitly requested is a separate prompted condition. Early vague resemblance never earns a strict pass.

Q intentionally contains a mix: historical known-answer use, new necessary clarifications, inactive-field exclusion and current-context behavior controls. Incident q-avoid-environment uses staging supplied by the current configuration description; experiment q-avoid-instrument uses K4 supplied by the current request. They are not independently memory-retention evidence. The case matrix marks every affected case. Executing and using answers still requires valid world prerequisites, including retained identity evidence for identity-dependent work.

- **Q memory dependence:** Incident q-avoid-environment (staging) and experiment q-avoid-instrument (K4), where present, can be answered from current task/contract. Exact affected case IDs are listed in matrix[].memoryReliance.QCurrentContextControls. These are behavior controls, not independent evidence of memory retention. Other historical Q fields still require memory; overall task use also depends on delivered mapping/artifact prerequisites.
- **Unavailable strata:** Easy unresolved incident and experiment have no active necessary questions or fresh-work milestones. Q/W balanced ratings are unavailable there; do not invent pass credit or silently drop cases. Current strict full-bank aggregation will remain unavailable for those metrics unless a new authoring/scoring version explicitly defines the relevant strata.
- **Conditional branch calibration:** Demanding conditional discriminator values activate the dependent branch in every authored case. Tests cover timing and unrelated inactive-field exclusion, not a balanced study of active and inactive conditional outcomes.
- **Memory controls:** Identity-withheld controls isolate D/identity-dependent work. Release unresolved instead removes historical license result and all originating echoes, allowing a new current check without inventing historical execution. These narrow controls do not prove every H/Q/W requirement independently relies on memory.
- **Difficulty and generalization:** Noise is explicitly marked unrelated; 27 cells reuse three causal families. Structural validation and local witnesses establish feasibility, not live difficulty, held-out generalization, model semantic reliability or product benefit.
- **Review authority:** The initial authoring review was independent of the implementing agents; this reviewer subsequently implemented confirmed corrections and focused regression tests. This is agent-assisted verification, not independent human certification. It is not the pending human audit of saved v5 results and does not rescore, replace or validate prior outcomes.

## Complete case coverage

Counts are H; Q necessary/avoid; W reuse/fresh; one strict D identity decision per mode. Guards are included in avoid/reuse counts. NA means no applicable opposing stratum, never an observed zero or automatic pass.

| Case | H | Q N/A | W R/F | Primary terminal | Withheld terminal | Plan operations | Current-context Q controls |
|---|---:|---:|---:|---|---|---:|---|
| v5-incident-easy-same | 12 | 2/2 | 3/2 | complete | blocked | 7 | q-avoid-environment=staging |
| v5-incident-easy-distinct | 12 | 2/2 | 3/2 | complete | blocked | 7 | q-avoid-environment=staging |
| v5-incident-easy-unresolved | 12 | 0/2 | 1/0 | blocked | blocked | 3 | none |
| v5-incident-intermediate-same | 16 | 3/3 | 5/3 | complete | blocked | 10 | q-avoid-environment=staging |
| v5-incident-intermediate-distinct | 16 | 3/3 | 5/3 | complete | blocked | 10 | q-avoid-environment=staging |
| v5-incident-intermediate-unresolved | 16 | 1/3 | 2/1 | blocked | blocked | 5 | none |
| v5-incident-demanding-same | 20 | 3/4 | 5/4 | complete | blocked | 12 | q-avoid-environment=staging |
| v5-incident-demanding-distinct | 20 | 3/4 | 5/4 | complete | blocked | 12 | q-avoid-environment=staging |
| v5-incident-demanding-unresolved | 20 | 2/4 | 2/1 | blocked | blocked | 6 | none |
| v5-experiment-easy-same | 12 | 2/2 | 3/2 | complete | blocked | 7 | q-avoid-instrument=K4 |
| v5-experiment-easy-distinct | 12 | 2/2 | 3/2 | complete | blocked | 7 | q-avoid-instrument=K4 |
| v5-experiment-easy-unresolved | 12 | 0/2 | 1/0 | blocked | blocked | 3 | none |
| v5-experiment-intermediate-same | 16 | 3/3 | 5/3 | complete | blocked | 10 | q-avoid-instrument=K4 |
| v5-experiment-intermediate-distinct | 16 | 3/3 | 5/3 | complete | blocked | 10 | q-avoid-instrument=K4 |
| v5-experiment-intermediate-unresolved | 16 | 1/3 | 2/1 | blocked | blocked | 5 | none |
| v5-experiment-demanding-same | 20 | 3/4 | 5/4 | complete | blocked | 12 | q-avoid-instrument=K4 |
| v5-experiment-demanding-distinct | 20 | 3/4 | 5/4 | complete | blocked | 12 | q-avoid-instrument=K4 |
| v5-experiment-demanding-unresolved | 20 | 2/4 | 2/1 | blocked | blocked | 6 | none |
| v5-release-easy-same | 12 | 2/2 | 3/2 | complete | complete | 7 | none |
| v5-release-easy-distinct | 12 | 2/2 | 3/2 | complete | complete | 7 | none |
| v5-release-easy-unresolved | 12 | 2/2 | 3/2 | complete | complete | 7 | none |
| v5-release-intermediate-same | 16 | 3/3 | 5/3 | complete | complete | 10 | none |
| v5-release-intermediate-distinct | 16 | 3/3 | 5/3 | complete | complete | 10 | none |
| v5-release-intermediate-unresolved | 16 | 3/3 | 5/3 | complete | complete | 10 | none |
| v5-release-demanding-same | 20 | 3/4 | 5/4 | complete | blocked | 12 | none |
| v5-release-demanding-distinct | 20 | 3/4 | 5/4 | complete | blocked | 12 | none |
| v5-release-demanding-unresolved | 20 | 3/4 | 5/3 | blocked | blocked | 11 | none |

## Source-grounding conclusions

- Incident: Mira owns the staging service; Noah proposes the mitigation with joint pool/DNS reasons; Ada limits and later changes trial authorization. Tool receipts alone establish measurements and execution. A correction and a production proposal do not withdraw or extend approval.
- Experiment: Chen states sensitivity AND staffing criteria; Relay and Beacon outcomes require actual comparison receipts. Omar limits adoption and later batch size; Lena prepares the draft; the attribution correction is not reversal. Shared platform does not make pediatric and adult questions identical.
- Release: Priya now explicitly declares the Aurora locale-validation producer role before maintenance; Tomas supplies reviewer availability and Elise supplies scoped approval. Locale validation/signoff is not distribution authority. No conversation label or chosen window is used to infer an unrecorded role.

All hidden unknown-field response values belong to the deterministic scripted respondent; they are not treated as already answered history. Every H criterion maps to a public reconstruction question. Citations are an independent compliance diagnostic; unsupported meaning still fails.

## Source binding

Review source-manifest SHA-256: `e190673613ac09037169353a78a0f5e4ba5eff98349f2f25868f85b7ca9bacbc`. Exact file-byte hashes are stored in both authoring-review.json and semantic-review-evidence.json. Any changed reviewed source invalidates this review gate. This is an authoring review of prospective benchmark behavior, not approval to run evaluated agents, a claim of product superiority, or a replacement for the required human outcome audit.
