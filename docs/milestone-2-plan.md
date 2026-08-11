# Milestone 2 implementation plan

Milestone 2 introduces filesystem mutation only after each preceding safety gate
is reviewable and green. Every stage uses behaviour-first tests and a conventional
commit; no stage broadens Tauri permissions or accepts native paths from the
WebView.

The normative execution decisions are in
[ADR 0001](decisions/0001-journaled-execution-contract.md).

Current status: Stage 2A is complete. Stage 2B is next; filesystem mutation
remains unavailable.

## Stage 2A — protocol model

- model forward, rollback, terminal, and reconciliation states in pure Rust;
- reject invalid, duplicated, out-of-order, and post-terminal journal records;
- calculate deterministic two-phase and reverse rollback order;
- cover swaps, cycles, case-only plans, cancellation boundaries, forward failure,
  rollback failure, and prepared-without-completed interruption;
- retain the read-only Tauri surface.

Exit: pure state replay has behavioural coverage and no filesystem operation is
reachable.

## Stage 2B — journal codec

- define a versioned, length-delimited, checksummed journal framing format;
- use create-new files and append-only writes;
- synchronise the header, prepared/completed pairs, and terminal records;
- reject truncation, corruption, unknown versions, oversized frames, and invalid
  native-name encodings without lossy conversion;
- keep path-bearing data local and path-free by default in errors and logs.

Exit: golden fixtures replay across supported versions and crash-boundary tests
classify incomplete state without mutation.

## Stage 2C — platform execution boundary

- add execution-grade Windows `FILE_ID_INFO` snapshots;
- add a handle-based no-replace rename primitive;
- allocate same-parent temporary names without replacement races;
- provide an injectable filesystem-operation trait for deterministic failures;
- test destination races, stale identity, access denial, reparse points, read-only
  entries, hard links, and unsupported filesystems.

Exit: Linux adapter tests and required Windows CI prove no unrelated destination
can be overwritten.

## Stage 2D — executor and rollback

- freeze current validated plans by plan ID and source generation;
- serialise execution through one mutation lock and blocking worker;
- write prepared/completed records around every forward step;
- stop cancellation at a step boundary and roll back in reverse order;
- preserve rollback failures as `RecoveryRequired`;
- expose structured, path-free progress and error codes.

Exit: injected failure after every operation produces a truthful terminal or
recovery-required state, with no silent partial success.

## Stage 2E — startup recovery and Rename Ledger

- discover and validate incomplete local journals at startup;
- project completed, rolled-back, interrupted, and damaged transactions;
- reconcile ambiguous prepared steps using native entry identity;
- require explicit inspection before retry, forward recovery, or rollback;
- add accessible progress, cancellation, and recovery interaction states.

Exit: keyboard-only recovery and packaged Windows interruption tests pass, and the
UI never receives native paths.

## Stage 2F — Undo

- derive Undo as a new validated plan from a completed transaction;
- revalidate post-rename identities and original destination availability;
- execute Undo through the same journaled two-phase protocol;
- reject stale, ambiguous, partially recovered, or superseded transactions.

Exit: Undo succeeds only for fully revalidated transactions and cannot overwrite
an unrelated entry.

## Publication gate

- `pnpm verify:full` passes locally;
- Linux and Windows Rust suites include real temporary-filesystem coverage;
- Windows desktop build and packaged interruption smoke tests pass;
- security review covers IPC, journal parsing, native identity, no-replace calls,
  local data handling, and recovery authorization;
- remaining platform limitations are documented before enabling Apply.
