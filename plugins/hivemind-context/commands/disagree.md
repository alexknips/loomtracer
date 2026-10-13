---
name: disagree
description: Push back on a decision by description, never by id. Records dissent; never silently overrides.
argument-hint: '"description of the decision" --reason "why you disagree"'
allowed-tools: Bash(${CLAUDE_PLUGIN_ROOT}/scripts/disagree.sh:*), Bash(${CLAUDE_PLUGIN_ROOT}/scripts/verify.sh:*)
disable-model-invocation: true
---

Record disagreement with a decision, resolved by description. This is a
WRITE verb — the ambiguity gate is strict. Runs `disagree`:

`$ARGUMENTS`

Run the plugin helper:

```bash
${CLAUDE_PLUGIN_ROOT}/scripts/disagree.sh $ARGUMENTS
```

If the description does not resolve to exactly one decision, the CLI
returns the candidate list and performs **no write**. Show the candidates
and re-invoke with `--pick N` (or `--decision <id>`) once you're certain —
never guess which decision to contest.

**Take it upstream.** A decision often rests on a prior decision, and your
disagreement may be with that premise. Before you record it, run
`${CLAUDE_PLUGIN_ROOT}/scripts/verify.sh "<the decision>"`, show the user what it
rests on, and ask whether the disagreement is with this decision or with a
premise. If it is with the premise, run `/hivemind-context:disagree` (or
`supersede`) on the premise too; the disagreement you record here is never
dropped or swapped for it. After the write, a newer hivemind lists a
`raise_upstream:` line for each premise this decision rests on that still holds,
with how many other decisions rest on it — a hint to ask the same question, not
a reason to undo what you recorded.
