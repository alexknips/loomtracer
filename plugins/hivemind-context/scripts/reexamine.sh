#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat >&2 <<'USAGE'
Usage:
  reexamine.sh [--decision "description of a decision"] [--id ID] [--pick N]
               [--project HANDLE] [--all] [--limit N] [--cursor C]

"What rests on something that changed, and has nobody looked again?" Lists the
open re-examine items: decisions that rest on a premise (a decision they follow
from, or an assumption or bet) which was superseded, rejected, contested or
refuted, and that nobody has said still hold. Each names the decision, the
premise, how it changed and when. --all adds the closed ones (who reaffirmed
them, and why). --decision narrows to one decision's items (resolved by
description, with the same ambiguity gate as the other verbs); --project asks
from one project. A long list says so (truncated, with the cursor to continue
from). To close an item, run reaffirm.sh. Runs:
  hivemind reexamine
USAGE
}

if [[ "${1:-}" == "--help" || "${1:-}" == "-h" ]]; then
  usage
  exit 0
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib.sh
source "$SCRIPT_DIR/lib.sh"

# Every argument is a flag, so there is no description to join. "$@" with no
# arguments is safe under `set -u` on macOS bash 3.2 (only an empty array is not).
hivemind_context_exec reexamine "$@"
