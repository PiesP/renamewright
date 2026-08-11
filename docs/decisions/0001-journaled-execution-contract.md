# ADR 0001: Journaled execution contract

- Status: Accepted for Milestone 2 implementation
- Date: 2026-08-11
- Scope: local, same-parent file renames on Windows-first desktop builds

## Context

Milestone 1 produces immutable, path-free preview plans but deliberately exposes no
filesystem mutation command. Milestone 2 must turn an approved current plan into a
recoverable transaction without allowing the WebView to provide paths or bypass
backend validation.

The execution boundary must handle swaps, cycles, case-only changes, cancellation,
process interruption, occupied destinations, stale source identity, and rollback
failure. A successful API call is not sufficient evidence of durability: journal
records and filesystem steps need an explicit ordering contract.

## Decisions

### Execution-grade plan

The backend freezes a separate execution-grade plan after the user reviews a
current preview. It contains:

- the opaque plan ID and exact source-registry generation;
- one entry per changed source, keyed by opaque source and parent IDs;
- native original and final name components;
- the admission fingerprint and the execution-time identity snapshot;
- a deterministic temporary name allocated within the original parent;
- an ordered two-phase schedule.

This plan is Rust-owned and is not accepted from exported JSON or reconstructed
from WebView display strings. The WebView may submit only the current opaque plan
ID. The backend refuses blocked, unchanged-only, superseded, or already-consumed
plans.

### Identity and revalidation

Windows execution opens the directory entry without following a reparse point and
queries `FILE_ID_INFO` through `GetFileInformationByHandleEx`. The authoritative
identity is the pair of volume serial number and 128-bit file ID. Metadata such as
size and modification time remains a change signal but cannot substitute for the
file ID when execution is enabled.

If the filesystem cannot provide a stable execution-grade identity, execution is
blocked. Renamewright does not silently downgrade to path-plus-timestamp identity.
Linux test adapters may use device and inode identifiers, but that does not relax
the Windows acceptance contract.

Immediately before every forward or rollback rename, the executor reopens the
expected source entry and confirms its identity. It also confirms that the target
name is absent, unless the target is the transaction-owned entry expected at that
step.

### No-replace primitive

`std::fs::rename` is forbidden inside the executor because its contract permits
replacement of an existing destination. Windows execution uses
`SetFileInformationByHandle` with `FileRenameInfo` or `FileRenameInfoEx` and never
sets `ReplaceIfExists` or `FILE_RENAME_REPLACE_IF_EXISTS`.

The primitive accepts an already validated source handle and a same-parent native
name. It returns a typed `DestinationExists` result when the target is occupied.
There is no check-then-rename fallback, no cross-volume move, and no arbitrary
destination path supplied by the frontend.

### Two-phase schedule

Every changed entry moves from its original name to a transaction-owned temporary
name before any entry moves to its final name. This makes swaps, longer cycles, and
case-only changes use the same protocol:

1. source-to-temporary steps in deterministic source-ID order;
2. temporary-to-final steps in the same order.

Temporary names are valid Windows components, are generated from a transaction ID
and source ID, and are admitted only after an atomic no-replace reservation check.
They are never shown as user-authored destinations.

### Journal format and durability

Each transaction owns a versioned append-only journal created with create-new
semantics in the application data directory. The header contains the complete
native original, temporary, and final graph plus execution-grade identities. Paths
and names in journals are local sensitive data and are never sent to the WebView,
telemetry, crash reporting, or default logs.

The writer appends one framed record at a time with a schema version, monotonically
increasing sequence number, record kind, payload length, and checksum. Unknown
versions, truncated frames, checksum failures, duplicate sequence numbers, and
impossible transitions stop automatic execution and require inspection.

The required ordering for a filesystem step is:

1. append `StepPrepared` and call `sync_all` on the journal;
2. execute one no-replace rename;
3. append `StepCompleted` with the observed post-step identity and call `sync_all`;
4. publish progress only after the completed record is durable.

The journal header is synchronised before the first step. Terminal completion or
rollback records are also synchronised. Directory-entry durability needs a
platform-specific implementation and power-loss tests; until those tests pass on
supported Windows filesystems, the product promises crash-aware reconciliation,
not power-loss atomicity.

### Rollback, cancellation, and recovery

Forward failure or cancellation at a step boundary enters rollback. Completed
steps are considered in reverse order. A rollback rename runs only when the
current entry identity matches the transaction-owned expected identity and the
rollback destination is free.

A rollback error is appended and synchronised, then the transaction becomes
`RecoveryRequired`; it is never reported as a clean rollback. Cancellation is
`RollbackRequested` with a user-cancellation cause, not an immediate terminal
state.

At startup, replay classifies each journal as:

- `Completed` or `RolledBack`: terminal and inspectable;
- `ForwardPending`: every prepared step is completed and the next step is known;
- `RollbackPending`: rollback started and the next reverse step is known;
- `ReconciliationRequired`: a prepared step lacks a matching completion, a frame
  is damaged, or observed identity is ambiguous.

Prepared-but-not-completed steps are never guessed from the record stream alone.
Recovery compares the original, temporary, and final entry identities before
offering a forward or reverse action. No recovery action runs automatically at
startup.

### IPC and concurrency

Execution runs on one dedicated blocking worker guarded by a session mutation
lock. Only one transaction may mutate the filesystem at a time. IPC errors and
progress events use structured path-free codes; full native paths remain available
only in explicit local diagnostics.

The initial execution command remains absent until the platform primitive,
journal writer, replay validator, injected-failure suite, Windows integration
tests, and recovery UI contract all pass review.

## Rejected alternatives

- Direct `std::fs::rename`: can replace an existing destination.
- Check target absence and then rename: introduces a time-of-check/time-of-use
  overwrite race.
- One-step rename ordering: cannot safely represent swaps and cycles.
- Metadata-only identity: can mistake a replaced entry for the admitted source.
- Best-effort buffered logging: cannot establish whether a step was intended or
  completed after interruption.
- Automatic startup recovery: acts on ambiguous filesystem state without user
  review.
- WebView-supplied plan documents or paths: violates the native path boundary.

## Implementation gates

1. Pure journal replay and rollback planning with exhaustive transition tests.
2. Framed journal codec with truncation and corruption fixtures.
3. Injectable no-replace filesystem boundary and failure at every step.
4. Real temporary-directory tests on Linux plus Windows identity/no-replace tests.
5. Startup recovery projection and path-free IPC contract.
6. Rename Ledger, explicit Apply, cancellation, inspection, and Undo UI.
7. Packaged Windows interruption and recovery smoke tests.

## References

- [Microsoft `FILE_ID_INFO`](https://learn.microsoft.com/en-us/windows/win32/api/winbase/ns-winbase-file_id_info)
- [Microsoft `FILE_RENAME_INFO`](https://learn.microsoft.com/en-us/windows/win32/api/winbase/ns-winbase-file_rename_info)
- [Microsoft `SetFileInformationByHandle`](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-setfileinformationbyhandle)
- [Rust `std::fs::rename`](https://doc.rust-lang.org/std/fs/fn.rename.html)
- [Rust `File::sync_all`](https://doc.rust-lang.org/std/fs/struct.File.html#method.sync_all)
