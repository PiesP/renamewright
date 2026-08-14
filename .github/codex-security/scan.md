# Renamewright Codex Security scan instructions

Review this repository as the native Windows file-renaming workbench described
by the knowledge base. Prioritize exploitable security defects over generic
robustness, style, maintainability, or speculative platform concerns. Require a
current source-to-sink trace before treating a candidate as a vulnerability.

For every candidate finding:

1. Identify the attacker-controlled or concurrently changed source and the
   exact reachable authority or resource sink.
2. Trace opaque IDs, plan generation, confirmation, identity revalidation,
   parent confinement, no-replace execution, journaling, cancellation,
   rollback, Recovery, reconciliation, and Undo across the complete path.
3. Cite current files and lines. State the minimal triggering state, required
   user interaction, supported platform, build features, and whether any
   filesystem mutation occurs.
4. Explain the violated property and calibrate severity after current bounds,
   native OS enforcement, recovery behavior, and release controls.
5. Reject candidates that are unreachable, test-only, path-free, bounded,
   fail-closed, prevented by the mutation lock or identity boundary, or merely
   duplicate a dependency advisory without a Renamewright-specific path.

Concentrate discovery on:

- Lossy native-name or path conversion, opaque-ID confusion, stale generations,
  plan-ID reuse, case-only and occupied-name behavior, and parent escape.
- TOCTOU windows, source replacement, reparse-point traversal, destination
  races, no-replace semantics, directory handling, and Windows handle identity.
- Journal framing, size/count bounds, corruption and torn tails, replay state,
  durable ordering, recovery decisions, Undo lineage, and reconciliation.
- Confirmation freshness, mutation-task lifecycle, overlapping operations,
  cancellation boundaries, lock ownership, detached threads, and shutdown.
- Rule, regex, preset, export, batch, trace, automation protocol, accessibility,
  and virtualized UI inputs that can amplify CPU, memory, disk, or retained data.
- Feature and artifact separation between production and automation, workflow
  trust, dependency installation, action pins, SARIF, packaging, checksums,
  SBOMs, release tags, signing, and secret scope.

Do not report browser, WebView, backend, SQL, SSRF, CSRF, account, session,
multi-tenant, plugin, shell, updater, or telemetry issues without first showing
that current source contains the corresponding authority. Do not treat a
display-only native filename, user-confirmed local selection, or source-bound
disposable test fixture as remote attacker control.

Coverage must explicitly list unverified Windows and packaged-runtime areas,
including NTFS/ReFS and Win32 behavior, real Explorer drag/drop, UI Automation,
IME, DPI/high contrast, process RSS, antivirus, signing, and any feature or
credential unavailable in the scan environment. Incomplete runtime evidence
must remain deferred runtime coverage rather than being reported as complete.
