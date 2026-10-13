---
name: reexamine
description: What rests on something that changed and has not been looked at again? Lists the open re-examine items.
argument-hint: '[--decision "description of a decision"] [--project HANDLE] [--all] [--limit N]'
allowed-tools: Bash(${CLAUDE_PLUGIN_ROOT}/scripts/reexamine.sh:*)
disable-model-invocation: true
---

List the decisions that rest on a premise which was superseded, rejected,
contested or refuted, and that nobody has said still hold. This is a read:
nothing is written. Runs `reexamine`:

`$ARGUMENTS`

Run the plugin helper:

```bash
${CLAUDE_PLUGIN_ROOT}/scripts/reexamine.sh $ARGUMENTS
```

Each item names the decision, the premise it rests on, how that premise
changed and when. `--all` adds the closed ones, with who reaffirmed them and
why. `--decision "<description>"` narrows to one decision (quote it; an
ambiguous description lists candidates, re-run with `--pick N`); `--project`
asks from one project. A long list stops at `--limit` and says so, with the
cursor to continue from (`--cursor`).

Report what you find as it is: an item that stays open is a fact about the
record, not a mark against anyone. To close one, look at the decision again and
run `/hivemind-context:reaffirm` if it still holds, or `disagree` /
`supersede` it if it does not.

Needs a hivemind newer than v0.7.0; an older CLI says it has no `reexamine`
command.
