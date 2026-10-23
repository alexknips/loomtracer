#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat >&2 <<'USAGE'
Usage:
  disagree.sh "description of the decision" --reason "why you disagree"
              [--decision ID] [--pick N] [--topic T]

  disagree.sh '#2' --reason "..."   # resolve candidate #2 from a prior ambiguous output

Push back on a decision by description, never by guessing an id. This is a
WRITE verb: the ambiguity gate is strict here. If the description does not
resolve to exactly one decision, the CLI returns the candidate list and
performs NO write — re-invoke with --pick N (or --decision <id>) once you
have picked the right one; never guess.

A decision often rests on a prior decision, and your disagreement may be with
that premise. Run verify.sh on it first, show what it rests on and ask whether
the disagreement is with the decision or with a premise. A newer loomtracer also
adds a "raise_upstream:" line per premise the decision rests on that still holds,
with how many other decisions rest on it; the disagreement you recorded stands
either way. Runs:
  loomtracer disagree
USAGE
}

if [[ "${1:-}" == "--help" || "${1:-}" == "-h" ]]; then
  usage
  exit 0
fi

# These verbs are resolved by a description, so no argument at all is a usage
# error. Say so here rather than reaching the CLI with an empty argument list.
if [[ $# -eq 0 ]]; then
  usage
  exit 2
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib.sh
source "$SCRIPT_DIR/lib.sh"

loomtracer_context_join_description "$@"
# ${arr[@]+...}: macOS bash 3.2 treats expanding an empty array under `set -u`
# as an unbound variable, and HC_ARGS is empty for an empty description.
loomtracer_context_exec --write disagree ${HC_ARGS[@]+"${HC_ARGS[@]}"}
