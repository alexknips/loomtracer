---
name: capture-decision
description: Capture one HiveMind decision in the configured ledger using the legacy kind-locked path
argument-hint: '--title "..." --rationale "..." --topic-keys topic[,topic] --options "Label[,Label]" [--chose "Label"] [--decided-by actor-id] [--delegated-by human:name] [--still-proposed] (--rests-on-decision "..." | --rests-on-evidence "..." --evidence-source "..." | --rests-on-assumption "..." | --bet ["..."]) (--binds "..." | --binds-nothing) (--undo-cost "..." | --cheap-to-undo) [--project handle] [--source agent|human]'
allowed-tools: Bash(${CLAUDE_PLUGIN_ROOT}/scripts/capture-decision.sh:*)
disable-model-invocation: true
---

`/hivemind-capture:capture` is the primary capture command. This legacy command
keeps existing decision-only usages working and always dispatches with
`--kind decision`.

Capture exactly one HiveMind decision from the supplied arguments:

`$ARGUMENTS`

Run the plugin helper:

```bash
${CLAUDE_PLUGIN_ROOT}/scripts/capture-decision.sh $ARGUMENTS
```

The helper prints a confirmation and a follow-up
`/hivemind-capture:query-decisions` command scoped to the recorded actor.

Default to `--source agent`, which records `actor_id=agent:claude:<name>`
(`HIVEMIND_AGENT_SESSION` when set, then Gas City's `GC_AGENT`/`GC_ALIAS`, otherwise the
raw session id, so with none of them each session is its own actor) and `source=agent`. Use `--source human` only when the user
explicitly asks you to record their decision as a human write.

`--chose option` means the decision was already made: it self-accepts
immediately (or accepts from `--decided-by <actor-id>` when someone else
decided). Pass `--still-proposed` instead to float a leaning that still
awaits someone else's decision.

Write `--title` as one plain sentence a stranger understands, and keep
`--rationale` readable without the tracker. A tracker or ticket id (a bead,
Jira, or GitHub issue) goes in `--evidence-source` or a linked reference —
never in the title, and not as the rationale's subject.

When you decided within a scope a human explicitly delegated to you, add
`--delegated-by <human:name>` so the record shows the delegation rather than an
agent deciding alone. It requires `--chose` and an agent actor.

The confirmation also names the decision's project (`project: billing
(folder_marker)`), worked out from the working folder; pass `--project <handle>`
only when you were told which project. When it says the decision was saved to
the personal project and the folder is not attached to a project yet, tell the
user, with the attach hint it prints.

Every decision must say what it rests on: `--rests-on-decision "<description>"`
(a decision we already made, or `'#N'` / the id from a consult), `--rests-on-evidence
"<what>" --evidence-source "<where>"` (something observed), `--rests-on-assumption
"<statement>"` (something we assume), or `--bet ["<statement>"]` (nothing yet).
A capture that names none exits 2 and writes nothing; add the grounding and
re-run, never drop the capture. The decider's own words are not a grounding:
they go in `--quote` with `--question`, and `--confidence` only when those words
state it. `/hivemind-capture:capture` documents the four answers in full.

It also asks what the decision binds and what undoing it would cost, and both
answers are required: `--binds "<what>"` or `--binds-nothing`, and `--undo-cost
"<what>"` or `--cheap-to-undo`. A choice is a decision when it binds something
beyond the edit in front of you or is costly to undo: typing "apple" for "Apple"
is not one, merging tonight despite a failing check is. A capture that answers
`--binds-nothing --cheap-to-undo` is still recorded, marked below the decision
floor; a request goes to your tracker. A capture that leaves either answer out
exits 2 and writes nothing.

This command is a write-layer path only: it does not query, rank, summarize, or
infer related decisions itself.
