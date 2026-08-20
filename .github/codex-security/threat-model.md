# Renamewright security context

## Architecture and protected assets

Renamewright is a Windows-first native Rust batch-renaming workbench built with
`eframe` and `egui`. The product has no browser runtime, account, telemetry,
cloud sync, updater, plugin system, shell integration, or required runtime
network access. Planning, execution, recovery, and Undo are backend-owned.

Protect these assets:

- The identity, contents, names, locations, and availability of user-selected
  files and directories.
- The confidentiality of native paths, journal name graphs, file metadata, and
  source-bound acceptance evidence.
- The integrity of the exact reviewed plan, native confirmation, mutation lock,
  durable journal, Recovery state, Undo lineage, and no-replace execution.
- The integrity and availability of source, dependencies, workflows, portable
  release artifacts, checksums, SBOMs, and future signing boundaries.

## Trust boundaries and attacker-controlled inputs

- Picker and drag/drop paths, native filenames, filesystem metadata, directory
  entries, reparse points, and concurrent filesystem changes are untrusted.
- Rule requests, regexes, sequence values, per-source overrides, preset files,
  CSV/JSON export destinations, and UI or accessibility actions are untrusted.
- Journal files are untrusted even though Renamewright created them originally;
  they may be truncated, corrupted, replaced, duplicated, or locally modified.
- Native confirmation is a security boundary. UI projections and automation
  receive opaque IDs and display data, never mutation authority or native paths.
- The feature-gated automation listener and built-in path-free profiles are
  test-only input boundaries. Automation is read-only and cannot admit native
  sources, persist presets, create journals, or invoke filesystem mutations.
  Production builds must exclude its markers and entry points.
- GitHub Actions, dependency locks, packaging scripts, release tags, checksums,
  SBOM generation, and future signing credentials form the build boundary.

## Required security properties

- Native paths remain `PathBuf` or `OsString` below the application boundary.
  UI, automation, exports, and ordinary logs remain path-free.
- Apply accepts only the exact current unblocked plan ID after native
  confirmation and fresh source identity validation. Plan IDs never wrap or
  repeat, including when untrusted journals contain extreme values.
- At most one mutation task is active and tracked. A second Apply, Recovery, or
  Undo cannot replace, detach, overlap, or outlive the tracked operation.
- Every rename stays within the admitted parent identity, uses a retained parent
  handle and a no-replace primitive, revalidates parent and source identity
  immediately before mutation, and records durable intent and outcome before
  authority advances. Legacy journals without parent identity cannot mutate.
- Failures and cancellation preserve a replayable journal and enter explicit
  rollback, Recovery, reconciliation, or blocked states without silent success.
  Recovery and Undo require fresh path-free inspection plus native confirmation.
- Explicit directories are treated as single entries. Descendants are not
  enumerated, ancestor and descendant selections are rejected, and reparse
  points are not followed.
- Source batches, journal discovery, frame sizes, rule counts, regex work,
  traces, exports, automation messages, and 10,000-entry UI projections remain
  bounded before allocation or iteration.
- Production artifacts exclude automation markers and bind checksums, SBOMs,
  acceptance evidence, and release metadata to the exact source SHA.
- Workflows use least privilege, immutable action pins, trusted executable
  installation, exact revisions, and no secret exposure to fork or Dependabot
  pull-request code.

## High-value review surfaces

- `crates/renamewright-core/src/`: plan construction, rules, diagnostics,
  execution protocol, replay semantics, sequence bounds, and native name graphs.
- `crates/renamewright-platform/src/`: source admission, identity, journal codec
  and writer, ledger discovery, execution, Recovery, Undo, and confinement.
- `crates/renamewright-application/src/lib.rs`: opaque DTOs, plan authority,
  confirmation, mutation serialization, journal selection, presets, and export.
- `crates/renamewright-windows-native/src/lib.rs`: reviewed Win32 handle and
  variable-length rename-buffer unsafe boundary.
- `crates/renamewright-app/src/`: native UI state, AccessKit projection,
  task lifecycle, automation feature gate, and production marker exclusion.
- `.github/workflows/`, packaging and acceptance PowerShell, Cargo manifests,
  lockfiles, tool digests, checksums, SBOMs, release, and signing configuration.

## Scope boundaries and coverage limits

Static review cannot prove NTFS or ReFS behavior, Win32 kernel semantics,
interactive Windows UI Automation, Korean IME composition, Explorer drag/drop,
DPI or high-contrast rendering, process RSS, antivirus interaction, code signing,
or packaged execution. Record these as deferred runtime coverage and use the
source-SHA-bound Windows acceptance workflow and owner-reviewed evidence.

Do not infer a network, server, browser, WebView, SQL, HTTP, account, session,
tenant, updater, plugin, or shell vulnerability unless current source contains
that authority. Test-only automation is reportable only when it can enter a
default production artifact or escape its explicit loopback, fixture-root, and
source-bound constraints. Generated reports, `target/`, acceptance artifacts,
and historical planning documents are not source-of-truth scan targets.

## Severity guidance

- **Critical:** reliable release/signing compromise or unattended arbitrary
  mutation outside the reviewed source set with broad irreversible impact.
- **High:** realistic path or identity confusion, replacement, journal bypass,
  confirmation bypass, production automation exposure, or secret-bearing CI
  compromise that can alter user files or published artifacts.
- **Medium:** bounded but reproducible loss of mutation tracking, recovery
  integrity, path confidentiality, or resource availability requiring user or
  operator recovery.
- **Low:** defense-in-depth gaps with narrow local impact or deliberate
  developer/operator prerequisites.

Calibrate every finding using a demonstrated source-to-sink path, current
feature reachability, user interaction, filesystem preconditions, persistence,
rollback behavior, platform enforcement, and affected scope.
