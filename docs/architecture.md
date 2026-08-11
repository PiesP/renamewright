# Architecture

## Selected stack

- **Application core:** stable Rust, pinned by `rust-toolchain.toml` once code is
  introduced.
- **Desktop shell:** Tauri 2.
- **UI:** Solid with TypeScript and Vite, managed by a pinned pnpm release.
- **Primary target:** Windows x86-64. Linux is used for core tests and optional
  development previews; macOS support is deferred.

Tauri is selected because the product needs a dense, accessible preview table
and rule editor while retaining a small native shell and a Rust-owned filesystem
boundary. Solid matches the existing workspace experience. A pure Rust GUI can
be reconsidered only if the vertical slice exposes a concrete WebView limitation;
it is not maintained as a parallel implementation.

## Repository shape

The implementation phase will introduce this structure incrementally:

```text
crates/
  rename-domain/   rules, plans, diagnostics, platform-neutral validation
  rename-fs/       discovery, snapshots, conflict graph, journal, executor
src/               Solid UI and typed IPC client
src-tauri/         Tauri composition root, commands, state, capabilities
tests/             cross-crate behavioural fixtures where unit tests are insufficient
```

`rename-domain` cannot depend on Tauri or frontend concepts. `rename-fs` depends
on the domain crate and contains narrowly isolated platform modules. `src-tauri`
adapts commands to use cases and owns application state; it does not contain rule
algorithms.

## Core model

The backend registry owns native paths. Frontend DTOs refer to entries through
opaque IDs and receive separate display fields.

```text
SourceSnapshot -> RulePipeline -> RenameIntent[] -> Validation -> RenamePlan
                                                            |
                                                            v
                                              JournaledExecutor -> Events
```

Important types include:

- `SourceId` and `SourceSnapshot`, including parent, native name, directory-entry
  identity, underlying file identity, type, size, and metadata fingerprint;
- versioned `RenameRule` values and a `RulePipeline` generation;
- `RenameIntent` with source ID, proposed component, and trace of rule effects;
- typed `Diagnostic` values with severity and affected IDs;
- immutable `RenamePlan` with a plan ID, source generation, conflict graph, and
  ordered execution steps;
- append-only `JournalRecord` and streamed `ExecutionEvent` values.

The UI may request a new proposal, but only the backend creates a plan and only a
current validated plan ID can be executed.

## Admission boundary

Native picker and drag/drop paths enter one admission use case. The backend:

1. rejects unsupported entry types;
2. rejects duplicate path admissions while retaining hard-linked directory
   entries and recording their shared underlying identity;
3. does not traverse directory symlinks and renames a symlink entry rather than
   following its target;
4. captures a source snapshot and stores native paths in the session registry;
5. returns opaque IDs and display projections;
6. requires later operations to use those IDs rather than arbitrary write paths.

The Tauri WebView receives no general shell permission and no broad filesystem
write capability. Capabilities are explicitly listed for the main window, remote
content is disabled, and v1 registers no updater or network plugin.

## Validation and conflict graph

Validation is split between portable rules and target-specific policy:

- empty or unchanged names;
- duplicate destinations under the target filesystem's comparison semantics;
- destinations occupied by entries outside the plan;
- reserved Windows device names and illegal component characters;
- trailing dot/space behaviour, component length, and unsupported path shape;
- case-only changes and rename cycles;
- source identity or metadata changed since the snapshot;
- symlink, hard-link, permission, and read-only warnings;
- confinement to the original parent directory in v1.

Comparison and display normalisation are never treated as the same operation.
Unicode normalisation changes filenames only through an explicit user rule.

## Execution protocol

Execution runs on one dedicated blocking worker and holds a session mutation lock.
The protocol is:

1. Revalidate every source and destination against the plan snapshot.
2. Allocate non-existing temporary names in each original directory and write a
   synchronised journal header containing the complete original, temporary, and
   final path graph.
3. Before each rename, append and synchronise a `StepPrepared` record.
4. Execute a platform `rename_no_replace` operation that cannot overwrite an
   unrelated destination.
5. Append and synchronise `StepCompleted`, then repeat for the source-to-temporary
   and temporary-to-final phases.
6. Mark the transaction completed, synchronise the journal, and publish the new
   source snapshot.

On failure, completed steps are walked in reverse and rolled back when current
identity still matches the journal. Rollback failure is recorded, never hidden.
Startup detects incomplete journals and offers inspection, retry, or safe recovery.
Recovery reconciles prepared-but-not-completed steps against current directory
entry identities before choosing a forward or reverse action.

Cancellation stops at a step boundary and enters the same rollback path. It is
not presented as instantaneous cancellation.

## Undo

Undo is a new validated plan generated from a completed journal. It is available
only when current entries still match the recorded post-rename identities and
their original destinations are free or are part of the same reversible graph.
The UI describes partial or unavailable undo explicitly.

Journals contain paths and therefore remain local application data. Logs use
entry IDs and structured error categories by default; path logging is opt-in for
diagnostics and never sent remotely.

## Dependency policy

Dependencies are added only when a vertical slice needs them. Likely categories
are serialization, typed errors, linear-time regular expressions, Unicode
normalisation, stable IDs, tracing, Tauri, and native dialogs. Property testing
and temporary directories belong in development dependencies.

Exact versions live only in Cargo and pnpm manifests and their lockfiles. New
releases observe the workspace's 24-hour cooling window, and dependency build
scripts or native code are reviewed before admission. CI will run advisory,
license, and source-policy checks after the initial manifests exist.

## Test architecture

- **Domain unit and property tests:** rule composition, Unicode, token expansion,
  numbering scopes, determinism, and diagnostic invariants.
- **Filesystem integration tests:** temporary directories, collisions, cycles,
  case-only changes, injected failures, rollback, crash journals, and stale plans.
- **Windows-specific tests:** reserved names, case-insensitive comparison, long
  paths, read-only entries, and packaging.
- **Frontend tests:** rule editing, preview filtering, keyboard operation,
  accessible states, IPC error presentation, and large virtualised tables.
- **Packaged smoke tests:** file admission, one multi-rule plan, Apply, journal,
  and Undo against a disposable fixture.

The executor is tested through an injectable filesystem-operation boundary so a
failure can be placed after any step. Real-filesystem tests remain mandatory for
platform semantics. The boundary exposes no-replace rename explicitly; portable
`std::fs::rename` is not accepted where the host may replace an existing target.

## Distribution boundary

The first release target is a signed Windows installer and a checksummed portable
artifact. Code signing requires a user-owned certificate or signing service and
will be requested only when release packaging begins. No certificate, GitHub
remote, or external service is required for the architecture and vertical slice.
