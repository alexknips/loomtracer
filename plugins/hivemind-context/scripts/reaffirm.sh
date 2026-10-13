#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat >&2 <<'USAGE'
Usage:
  reaffirm.sh "description of the decision" --because "why it still holds"
              [--premise "id or words of the premise that changed"]
              [--id ID] [--pick N] [--topic T]

  reaffirm.sh '#2' --because "..."   # resolve candidate #2 from a prior ambiguous output

"I looked again: it still holds." Close a decision's open re-examine item. A
decision that rests on a premise which was since superseded, rejected,
contested or refuted has an open item (reexamine.sh lists them); looking
again and finding it still holds is recorded here, with a one-line reason,
attributed to you. A later change to the premise reopens the item.

Resolved by description, never by guessing an id. This is a WRITE verb: the
ambiguity gate is strict. If the description does not resolve to exactly one
decision, the CLI returns the candidate list and performs NO write -- re-invoke
with --pick N (or --id <id>) once you have picked the right one; never guess.
With exactly one open item no --premise is needed; with several the CLI lists
them and writes nothing -- name one with --premise. Anything with nothing to
re-examine (the premise has not changed, it was already reaffirmed, the
decision does not rest on it, the decision no longer stands) is refused, exit
2, and says why. Runs:
  hivemind reaffirm
USAGE
}

if [[ "${1:-}" == "--help" || "${1:-}" == "-h" ]]; then
  usage
  exit 0
fi

# This verb is resolved by a description, so no argument at all is a usage
# error. Say so here rather than reaching the CLI with an empty argument list.
if [[ $# -eq 0 ]]; then
  usage
  exit 2
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib.sh
source "$SCRIPT_DIR/lib.sh"

hivemind_context_join_description "$@"
# ${arr[@]+...}: macOS bash 3.2 treats expanding an empty array under `set -u`
# as an unbound variable, and HC_ARGS is empty for an empty description.
hivemind_context_exec --write reaffirm ${HC_ARGS[@]+"${HC_ARGS[@]}"}
