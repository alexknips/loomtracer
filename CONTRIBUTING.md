# Contributing to Loomtracer

Welcome. Loomtracer is memory for decisions: what was decided, which options
lost and why, and who decided. Contributions are welcome from humans and from
AI agents — both are first-class actors in the project.

Before you start, please read:

- [`VISION.md`](VISION.md) — why Loomtracer exists and what it is for
- [`PRINCIPLES.md`](PRINCIPLES.md) — the constraints Loomtracer cannot trade away
- [`STRATEGY.md`](STRATEGY.md) — the active investment fronts that judge
  whether a change is on-target
- [`AGENTS.md`](AGENTS.md) — the standard of excellence every contributor
  is held to
- [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) — the three-layer
  architecture and other architectural decisions

## How work is tracked

Loomtracer uses beads for all work tracking. They live in the city's Dolt
server, not in files in this repo (`.beads/issues.jsonl` is a stale export).
Planning lives in beads; the level-1 docs above are the compass beads are
judged against.

- `bd ready` — list beads that have no blockers and are ready to be picked up
- `bd show <id>` — read a bead's description, acceptance criteria, and deps

## Picking up work

1. Pick a bead that has no blockers, whose description you understand and
   whose acceptance criteria you can meet.
2. Read the bead's description fully, including its "Principles cross-check"
   and "Quality cross-check" sections.
3. Open a branch named `polecat/<bead-id>` from `master`.
4. Implement the change. Keep the diff scoped to the bead — file follow-up
   beads for anything bigger that surfaces.

## Quality gates (PRINCIPLES §8)

Which gates you run locally depends on your diff. See
[docs/QUALITY_GATES.md](docs/QUALITY_GATES.md) for the full mandatory gate
set, proof-line format, and the docs-only exception's rationale — summary:

- **Docs-only diff** (no `.rs`, `Cargo.*`, or build/CI file changed): skip
  the cargo-based gates locally; there is no local gate.
- **Code diff**: run these locally before submitting:

  ```bash
  cargo fmt --check
  cargo clippy --locked --all-targets -- -D warnings
  cargo test --locked
  ```

The site and its docs live in
[`alexknips/loomtracer-site`](https://github.com/alexknips/loomtracer-site), not
here. If you add or change a CLI subcommand, flag, or MCP tool, regenerate the
reference there and open a matching pull request; see
[docs/QUALITY_GATES.md → Reference Docs](docs/QUALITY_GATES.md#reference-docs-live-in-the-site-repo).

UBS's critical-finding and warning-baseline gates run in CI only, regardless
of diff scope — do not run the local UBS wrapper as part of your submission
checklist.

The CI workflow re-runs the full gate set (including UBS) on every push. A
pull request that fails any gate does not merge. Quality is not a P3
follow-up — it is part of the work that made the bead claimable in the first
place.

If your change touches surfaces (CLI, MCP, future HTTP API), implement once
in the internal `commands` / `queries` module and expose identically through
every surface — see [docs/ARCHITECTURE.md → Surface
Uniformity](docs/ARCHITECTURE.md#surface-uniformity).

## Tests

Loomtracer has several test layers:

```bash
cargo test                              # full default test suite
cargo test --test golden                # golden query snapshots
cargo test --test golden -- --bless     # regenerate golden snapshots after intentional change
cargo test --test local_capture_demo -- --nocapture
cargo test --test slack_app -- --nocapture
cargo test --test seed -- --include-ignored
cargo test --test seed replay_smoke -- --nocapture
cargo test --test sqlite_wal_multiprocess shared_sqlite_ledger_accepts_concurrent_process_writes -- --nocapture
```

The Kuzu feature is optional, compiles bundled C++ (a cold build is slow), and
is not part of the default suite or of any per-change gate. The
[Kuzu nightly](docs/QUALITY_GATES.md#kuzu-nightly) builds and tests it on
`master`, so you do not need to build it to submit a change. To iterate on the
Kuzu adapter locally:

```bash
cargo test --features graph-kuzu kuzu -- --nocapture
```

## Submitting a change

1. Push your branch.
2. Open a pull request against `master`. Put the gate proof lines from
   [docs/QUALITY_GATES.md](docs/QUALITY_GATES.md) in its description, filled
   in honestly.
3. Reference the bead id in the PR description and in the commit message
   trailer (e.g., `(loomtracer-bead-id-XXXX)`).
4. The PR is reviewed; once gates pass on the rebased state and review is
   complete, it lands.

## License & sign-off

Loomtracer is licensed under [AGPL-3.0-only](LICENSE). By contributing
you agree your contribution is provided under the same license.

Outside contributors use the
[Developer Certificate of Origin](https://developercertificate.org/) to
track provenance. Sign off every commit with `-s`:

```bash
git commit -s -m "your message"
```

This appends a `Signed-off-by: Your Name <your@email>` trailer, which is
your assertion that you wrote the patch or otherwise have the right to
submit it under the project's license. Outside commits without a sign-off
will be asked to amend before merge. Commits by the maintainer and the
project's agents are exempt. A contributor license agreement check replaces
the sign-off before outside pull requests are accepted.

## Reporting issues

Open an issue on GitHub. For a security problem, do not put the details in a
public issue; report it privately as [`SECURITY.md`](SECURITY.md) describes.
