#!/usr/bin/env bash
# check-classify-queue.sh — Session hook script for Loomtracer classify-queue.
#
# Add to .claude/settings.json to be notified when unclassified batches are pending:
#
#   {
#     "hooks": {
#       "Stop": [{
#         "matcher": "",
#         "command": "loomtracer classify-queue list --json --limit 1 2>/dev/null | jq -e '(.pending_total? // ((.batches? // .) | length)) > 0' >/dev/null && echo '[loomtracer] classify-queue: batches pending — run /classify-queue to drain'"
#       }]
#     }
#   }
#
# Or use this script as the hook command:
#
#   "command": "/path/to/plugins/loomtracer-capture/scripts/check-classify-queue.sh"

set -euo pipefail

LOOMTRACER_DIR="${LOOMTRACER_DIR:-./loomtracer/}"

# A project nothing was recorded in has no queue, and the hook does nothing there. Look before
# asking: a loomtracer older than hivemind-odmnc made an empty ledger in the folder when it listed
# the queue, so this hook left a stray ./loomtracer/ in every project it ran in. A cell
# (LOOMTRACER_API_URL) or a shared database (LOOMTRACER_DATABASE_URL) names no folder: ask as usual.
if [ -z "${LOOMTRACER_API_URL:-}" ] && [ -z "${LOOMTRACER_DATABASE_URL:-}" ] \
  && [ ! -f "$LOOMTRACER_DIR/ledger.sqlite" ]; then
  exit 0
fi

# stderr is dropped: a loomtracer that refuses a folder with no ledger (exit 2) reads as an empty
# queue, which is what it is.
list() {
  loomtracer --loomtracer-dir "$LOOMTRACER_DIR" classify-queue list --json --limit "$1" 2>/dev/null
}

# Builds after the v0.7.0 release reply {batches, pending_total, truncated, budget} on a local
# ledger and a served cell alike, and `pending_total` is the whole queue's depth whatever the
# limit, so --limit 1 keeps the check cheap. The v0.7.0 release prints a bare list of batches
# for a local ledger and {batches, budget} (no pending_total) from a server: with no
# pending_total the batches are counted instead, asked for up to 1000.
total=$(list 1 | command -p jq -r '.pending_total? // empty' 2>/dev/null || true)
if [ -z "$total" ]; then
  total=$(list 1000 | command -p jq -r '(.batches? // .) | length' 2>/dev/null || echo 0)
fi

if [ "${total:-0}" -gt 0 ]; then
  echo "[loomtracer] classify-queue: $total batch(es) pending — run /classify-queue to drain"
fi
