#!/bin/sh
# Claude Code SessionStart hook (hivemind-wdwg): tell the agent, as additional context, to check
# the Loomtracer decision ledger before it acts and to record what is worth keeping: rules learned
# from failures, design choices later work must follow, reversals.
#
# Without it, an installed plugin with a connected MCP server went unused: 0 captures and 0
# recalls in 31 headless benchmark sessions, because the plugin's only hooks fire on
# AskUserQuestion, which an autonomous agent never calls. The text names the tools to call; it
# decides nothing and reads no ledger (the three layers stay apart: this is a fixed sentence).
#
# It fails open. With LOOMTRACER_DIRECTIVE_DISABLE set, or no `loomtracer` binary to run the plugin's
# MCP server, it prints nothing and exits 0. A ledger directory that does not exist yet is a new
# ledger, not a missing one: the MCP server creates it on the first capture, and the first
# session in a project is exactly where the directive is needed. Plain sh, no python3.
#
# Environment:
#   LOOMTRACER_DIRECTIVE_DISABLE  any value: print nothing.
#   LOOMTRACER_CAPTURE_BIN        the loomtracer binary; default is `loomtracer` on PATH.
[ -n "${LOOMTRACER_DIRECTIVE_DISABLE:-}" ] && exit 0
command -v "${LOOMTRACER_CAPTURE_BIN:-loomtracer}" >/dev/null 2>&1 || exit 0

# Keep it short: it is paid for in every session. The tests hold it under 600 characters (150 tokens
# at 4 characters per token). It says what is worth saving and what is not, because told only to
# "record decisions", agents saved a summary of each session and missed the rules that cost them
# something (hivemind-dd95). docs and the plugin README quote this text; a test keeps them equal.
cat <<'JSON'
{"hookSpecificOutput":{"hookEventName":"SessionStart","additionalContext":"Loomtracer ledger: before you act, call the `loomtracer` MCP tool `recall_decisions` (q = three or four key words about the task) and follow the earlier decisions that still hold. Call `capture_decision` (title, rationale = the why, options, grounding, binds, undo_cost) for: a rule learned from something that failed or cost you (save it the first time); a design or interface choice later work must follow; a reversal of an earlier decision (use `supersede_decision`). Not for routine status, session summaries, plans that hold only for this session, or a choice binding nothing and cheap to undo."}}
JSON
