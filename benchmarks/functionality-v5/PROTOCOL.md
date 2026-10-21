# Protocol and interpretation

## Workflow

An originating agent performs an actual simulated task and produces receipts. The resulting trace, rather than an evaluator-written ideal handoff, becomes the acquisition record. A maintenance agent updates its own Markdown files through four episodes: original reasoning/work, intervening noise, changed circumstances, and more noise. Notes may be selectively edited. Sources name multiple actors and conversations.

From one frozen checkpoint, independent sessions receive a reasoning reconstruction task, an ordinary action task, and an action task explicitly prompting identity assessment. A bounded broker exposes note lookup, eligible retained-source expansion, scripted questions, and simulated work. Reference/oracle and withheld-evidence controls diagnose whether an answer was possible and whether it actually depended on historical evidence. Controls are diagnostic conditions; they are excluded from primary quality and overhead comparisons.

For each successor task, the broker allows at most 16 operations. It enforces 32 KiB of distinct historical delivery across the task and 32 KiB of historical content in a single invocation; repeated history in subsequent invocations still costs reported input tokens. The broker trace is replayed subject to the 16-operation limit and a separate 500,000-byte adapter prompt guard. These are harness limits, not model context-capacity measurements. Byte limits constrain delivery only; they are never converted to token expenditure.

## Difficulty and cases

Every family has easy, intermediate, and demanding variants, crossed with same, distinct, and unresolved identity. H grows from 12 to 16 to 20 units, equally distributed across four dimensions. Q and W denominators depend on the applicable task branch, available receipts, and guards. The selected demanding screen has seven Q obligations per case and nine, nine, and eight W obligations respectively; the generated rubric supplies the exact denominator for every case. Later cases require more active inputs, dependencies, reuse decisions, and scope distinctions within the fixed operation allowance. The authored controls and feasible reference trajectories check that the requirements can be met together.

The present screen uses the three demanding cases listed in the README, one repetition each, and compact four-episode histories. It is not a long-context stress test or a measure of persistence over real elapsed months. Increasing unrelated messages, cross-conversation hops, or handoffs later must produce a separately frozen condition and retain missing-evidence controls.

## Strict scoring

An incomplete, ambiguous, unsupported, or materially contradictory answer fails its affected unit. Strictness is local: one failed unit does not erase correctly recovered independent units. Material contradictions propagate through the frozen dependency edges. Whole-chain and whole-task success are additional all-or-nothing diagnostics.

H averages unit success equally across premises, relationships, attribution, and changes, then equally across scenarios. Q balances necessary questioning against avoiding unnecessary questions; W balances valid reuse against necessary fresh work. For either paired measure, the rating is `100 × 2ab / (a + b)`, where `a` and `b` are the two success rates; either zero makes the result zero. Each separately scored question is charged once; the catch-all guard covers other unnecessary requests.

D balances same, distinct, and unresolved identity classes. Identity must be grounded in the underlying problem, not merely matching numbers or wording. Recognition after any substantive reuse, execution, or observation fails even if the final conclusion is right. Ordinary and prompted results remain separate. Correct recognition can coexist with unnecessary repeated work: that passes D but fails the applicable W unit.

The overall rating is `[H + (Q + W)/2 + D] / 3`. Publish whole-number ratings with denominators, completion, and review status. Missing difficulty bands or identity strata prevent a complete-suite aggregate; the three-case screen reports its demanding band. Pending judgments produce bounds, not invented definitive ratings.

Citation existence, range validity, and source visibility are separate diagnostics. A recovered meaning can pass H while a malformed citation fails citation compliance. A syntactically valid reference is not proof of semantic correctness. Unsupported claims and full-task completion remain visible beside component scores.

## Accounting

All saved invocations count exactly once, including failures. Report **CLI-reported token usage**, which includes instructions, framing, consumed context, and tool schemas. Report input, output, total, cached input, reasoning output, and uncached input when known. Cached input and reasoning output are subsets, not additional tokens. Unknown usage preserves known subtotals and prevents definitive overhead ratios.

Acquisition and memory maintenance shared by successor sessions are charged once. H execution is separate from the joint Q/W/D action execution. Mixed lookup-and-answer calls stay task execution; retrieval gets a separate category only when it is a distinct model invocation. Shared acquisition, canaries, controls, and grading appear in whole-experiment expenditure and are excluded from primary post-handoff arm overhead. Overlapping views are never added together.

For future matched comparisons, overhead against each baseline is `L − B`, `L / B`, and `100 × (L − B) / B`. A zero baseline permits only the absolute difference. Compare identical completed workloads, maintenance scope, protocol, and model settings, with quality and completion beside usage. Incomplete work cannot appear as a cheaper equivalent. This package's single-condition screen cannot establish those comparative ratios. No dollar estimates are produced.

## Review and reproducibility

Schema, source visibility, citations, frozen revisions, action order, receipts, and operation counts are checked deterministically. Semantic correctness uses blinded review with evidence, plus a stratified human audit and review of uncertain/disputed judgments. A review packet hides condition identifiers but may reveal memory representation. The initial authoring review and later outcome review serve different purposes.

Corpus, protocol, rubric, report, source hashes, runtime settings, prompt/adapter implementation, packet revisions, and usage are recorded. Missing returned-model identity leaves subscription comparisons exploratory. Implementation or corpus changes produce new snapshots; they do not rewrite existing evidence or accounting. New source-grounding repairs invalidate direct comparison with old affected cases unless the condition change is explicitly acknowledged.

First-loss diagnoses require evidence of successful earlier stages: acquisition, maintenance, delivery, reconstruction, and application. When an earlier stage cannot be established, the report must say so rather than assign blame from a final wrong answer.
