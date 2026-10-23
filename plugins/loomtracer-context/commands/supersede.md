---
name: supersede
description: Replace a decision with a new one, resolved by description, never by id.
argument-hint: '"description of the old decision" --title "..." --rationale "..." (--rests-on-decision "..." | --rests-on-evidence "..." --evidence-source "..." | --rests-on-assumption "..." | --bet) (--binds "..." | --binds-nothing) (--undo-cost "..." | --cheap-to-undo)'
allowed-tools: Bash(${CLAUDE_PLUGIN_ROOT}/scripts/supersede.sh:*), Bash(${CLAUDE_PLUGIN_ROOT}/scripts/verify.sh:*)
disable-model-invocation: true
---

Replace an existing decision with a new one, resolved by description. This
is a WRITE verb — the ambiguity gate is strict. Runs `supersede`:

`$ARGUMENTS`

Run the plugin helper:

```bash
${CLAUDE_PLUGIN_ROOT}/scripts/supersede.sh $ARGUMENTS
```

The replacement must say what it rests on: one of `--rests-on-decision "..."`
(a decision it follows from), `--rests-on-evidence "..."` with
`--evidence-source "..."` (something observed, and where),
`--rests-on-assumption "..."` (something assumed), or `--bet` (a declared bet).
A supersede that names none of these is refused ("a captured decision must say
what it rests on") and nothing is written.

It must also say what the replacement binds and what undoing it would cost:
`--binds "<what>"` or `--binds-nothing`, and `--undo-cost "<what>"` or
`--cheap-to-undo`. A choice is a decision when it binds something beyond the edit
in front of you or is costly to undo; typing "apple" for "Apple" is not one,
merging tonight despite a failing check is. A supersede that leaves either answer
out is refused and nothing is written; one that answers `--binds-nothing
--cheap-to-undo` is recorded, marked below the decision floor. A request goes to
your tracker.

`--chose <option>` means the replacement was already decided: it is accepted
from your actor right away. Add `--still-proposed` to keep an open
recommendation at `proposed`. The confirmation shows `new_status=`.

The replacement's project is worked out from the working folder (or named with
`--project <handle>`); with none found it inherits the project the old decision
is filed in now (after a move, the project it was moved to). A decision filed
in a personal project is replaced into your own personal project instead —
personal projects belong to one actor — and the confirmation says so.
The confirmation shows `project=` and `project_source=` (`inherited` for the
first case, `personal_fallback` for the personal-project exception).

If the description does not resolve to exactly one decision, the CLI
returns the candidate list and performs **no write**. Show the candidates
and re-invoke with `--pick N` (or `--old <id>`) once you're certain — never
guess which decision to supersede.

**Take it upstream.** A decision often rests on a prior decision, and what you
want to replace may be that premise rather than this decision. Before you
replace it, run `${CLAUDE_PLUGIN_ROOT}/scripts/verify.sh "<the old decision>"`,
show the user what it rests on, and ask whether the problem is with this
decision or with a premise. If it is with the premise, run
`/loomtracer-context:supersede` (or `disagree`) on the premise too; the
replacement you record here is never dropped or swapped for it. After the write,
a newer loomtracer lists a `raise_upstream:` line for each premise the replaced
decision rests on that still holds, with how many other decisions rest on it — a
hint to ask the same question, not a reason to undo what you recorded.
