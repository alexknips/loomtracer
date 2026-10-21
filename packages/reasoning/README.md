# Shared reasoning core

`src/index.ts` exports the source-preserving `ReasoningBundle` 0.3 model, validation, historical views, reconstruction packets, immutable merging, and chronological graph layout. The main application uses its `@reasoning` alias. No browser code imports the Node-only legacy converter.

`upgradeBundle(input)` explicitly converts 0.1/0.2 to private 0.3 namespaces while preserving all original records and IDs. `validateBundle(input)` performs the same conversion and returns diagnostics. A namespace is either private (`workspaceId` and `projectId` both null) or hosted (both present); `datasetId` equals the bundle ID.

Entries still resolve exact UTF-16 source spans. `record`/`audit` sources identify legacy records separately from chat. Legacy payloads preserve original rows, creators, status, approvals, timestamps, and metadata. Database creation time is a source log time, never an utterance time. Missing original authors remain unknown.

Portable grants and commitment receipts describe recorded history. Importing them does not authorize an actor or change a live commitment. Only server-validated actions and server-returned authoritative state may do that. Grant revocations and commitment changes append receipts; they do not mutate earlier decisions. Structural validation checks lineage and grant timing but cannot verify the identity of an imported signer.

An active legacy decision with an unverified original approver uses `legacy_state`: its existing authoritative state is grandfathered without claiming a human identity. The unknown actor and migration recording time are explicit; original ratification fields remain unchanged in the source. Only the operator migration can activate this basis, after comparing its source with the frozen database. Ordinary imports cannot. An imported historical approver is not silently relabeled as a current authenticated human.

Unmapped legacy relationship names are retained as `legacy:<exact-name>` with source/target roles. Cross-project edges are stored once in the source project with `externalEntryRefs`; `getView().externalConnections` exposes them separately from the local graph. Inverse navigation and evaluation packets retain them. Access to a pointer does not grant access to the other project's entry.

## Offline conversion

Run with Node 22.15 or later:

```sh
node --experimental-strip-types scripts/reasoning-convert.mjs snapshot.json private/conversion-output
```

The snapshot must contain `snapshotId`, a timezone-qualified `createdAt`, and complete `tables.nodes`, `tables.edges`, `tables.actors`, `tables.projects`, and `tables.audit_events` arrays. Additional tables and schema metadata participate in the snapshot hash. This is not a snapshot-acquisition command and does not certify backup consistency or restorability.

The command creates a fresh private directory containing one bundle per project and a conversion report with original-to-new mappings, row hashes, exact unassigned audit records, approval facts, and blocking concerns. Exit status 2 means preservation issues must be resolved before migration. It never writes to a database or activates authority. A third JSON argument may supply explicit `{workspaceId,nodeId,projectId,reason}` assignments for previously unassigned nodes; existing ownership cannot be overwritten. Every assignment remains in the report and source records.

Semantic reconstruction is a separate reviewed extraction, not a property of deterministic conversion. Original snapshots and conversion reports must be retained privately. This package does not invent rationale, turn challenges into withdrawals, or treat old approvals as new decisions.

## Checks

From this directory, run `../../node_modules/.bin/vitest run` and `../../node_modules/.bin/tsc --noEmit` after installing the web dependencies. Tests cover compatibility, exact source ranges, attribution, grouped and inverse relations, historical cutoffs, graph positions, immutable revisions, merge identity, authority lineage, and legacy preservation.
