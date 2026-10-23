---
name: reaffirm
description: Say a decision still holds after something it rests on changed. Closes its open re-examine item, with a reason.
argument-hint: '"description of the decision" --because "why it still holds" [--premise "id or words of the premise that changed"]'
allowed-tools: Bash(${CLAUDE_PLUGIN_ROOT}/scripts/reaffirm.sh:*)
disable-model-invocation: true
---

Look again at a decision that rests on something that changed, and — only if
you checked and it does still hold — say so, with a one-line reason. This is a
WRITE verb — the ambiguity gate is strict. Runs `reaffirm`:

`$ARGUMENTS`

Run the plugin helper:

```bash
${CLAUDE_PLUGIN_ROOT}/scripts/reaffirm.sh $ARGUMENTS
```

The statement is recorded with who you are and when, and the item reads
`reaffirmed` from then on, with your reason beside it. It is not a vote and it
is not a default: do not run it to clear a list. If the decision does *not*
still hold, say that instead — `disagree` or `supersede` it.

If the description does not resolve to exactly one decision, the CLI returns
the candidate list and performs **no write**. Show the candidates and
re-invoke with `--pick N` (or `--id <id>`) once you're certain — never guess
which decision to reaffirm. A decision with several open items needs
`--premise` (the CLI lists them and writes nothing). Anything with nothing to
re-examine — the premise has not changed, the item was already reaffirmed (the
refusal names who and when), the decision does not rest on that premise, or
the decision itself was superseded — is refused with the reason.

Needs a loomtracer newer than v0.7.0; an older CLI says it has no `reaffirm`
command.
