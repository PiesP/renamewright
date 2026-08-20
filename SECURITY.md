# Security Review Context

This file defines repository-wide threat-model and severity context for security
review. It does not replace the vulnerability reporting and supported-version
policy in `.github/SECURITY.md`.

## System and scope

Renamewright is a Windows-first, local desktop batch-renaming workbench. The
product is a native Rust `eframe`/`egui` application; its UI-independent
application service owns state and use cases while the shell retains native
dialogs and presentation. Explicit directory entries are admitted without
recursive discovery. Security review covers the native application, core,
application and platform crates, the Windows native boundary, journal and
recovery formats, test-only automation, build workflows, dependency controls,
and Windows portable artifacts.

The production application has no public network listener, account, telemetry,
cloud sync, updater, remote UI, shell integration, or user-script/plugin system.
Those absent surfaces reduce exposure; they do not make a reachable local
confidentiality, integrity, or availability failure non-reportable.

Important assets are:

- integrity and availability of files selected by the local user;
- confidentiality of native paths, journal contents, and local filesystem
  structure across every UI and automation boundary;
- truthful, recoverable journal state for every filesystem mutation;
- bounded resource use for attacker-controlled preview, IPC, preset, and journal
  data; and
- provenance and integrity of dependencies, workflows, and packaged artifacts.

## Threat model and trust boundaries

Treat every test-only inspection message, injected input event, profile name,
and automation root as untrusted. Automation is limited to built-in path-free
read-only profiles and must not admit native sources, persist presets, create
journals, or invoke filesystem mutations. Production artifacts must not compile
or start the custom inspection listener.

Treat filenames, rule and override text, preset documents, dropped entries,
filesystem races, and journal files as untrusted. A user may select a hostile
directory entry, local filesystem state may change after selection or
inspection, and an incomplete, torn, oversized, legacy, or locally modified
journal must fail closed. Native picker and drag/drop paths are trusted only as
Rust-side inputs to admission; they are not safe to disclose through path-free
UI or automation projections.

Native confirmation is the user-authorization boundary for new-plan Apply,
Recovery, and Undo. UI intent alone is not authorization. The filesystem state
observed before confirmation is not assumed to remain current afterward.

Third-party packages, downloaded build tools, GitHub Actions, lockfiles, SBOMs,
checksums, and uploaded artifacts form the software supply-chain boundary.

## Security invariants

- Native paths remain Rust-owned. UI models, inspection output, status text,
  plan JSON/CSV, and ordinary logs expose opaque IDs, display names, and
  structured codes rather than absolute paths or journal-native names.
- Export destinations are selected natively and opened with create-new
  semantics. No UI or automation message supplies mutation or journal paths.
- Counts, text fields, generated rule output, trace retention, journal frames,
  journal discovery, and serialized documents are bounded before large
  allocations or copies. Invalid and unsupported input fails closed.
- New-plan Apply is available only in the native shell for the exact current
  plan ID after native confirmation. The application service freezes and
  revalidates the Rust-owned plan, chooses the journal below its initialized
  data root, and holds the same mutation and cancellation boundary as Recovery
  and Undo. No path-bearing Apply command is registered.
- Default release builds contain no custom HTTP, TCP inspection, MCP, shell, or
  general filesystem automation API. Test automation requires a separately
  compiled feature, an explicit launch mode, loopback binding, a visible
  test-mode indicator, and a disposable absolute root. Its only selectable
  configuration is a bounded built-in path-free profile; source admission,
  preset persistence, journals, exports, Apply, Recovery, and Undo are disabled.
- Standard AccessKit and Windows UI Automation may expose and operate the visible
  interface under the current user's authority; it must not expose hidden native
  paths or bypass confirmation and revalidation.
- Recovery and Undo require a single mutation lock, a current path-free
  inspection bound to a backend-owned single-use authorization, native
  confirmation, and verification of the exact locked journal snapshot before
  mutation. Identity changes, occupied destinations, ambiguous prepared steps,
  damaged journals, stale expectations, or substituted journal files block
  mutation or require explicit reconciliation.
- Every rename mutation uses the reviewed append-and-sync journal protocol,
  handle-preserving source identity, atomic no-replace semantics, step-boundary
  cancellation, and explicit rollback/recovery outcomes. An unrelated existing
  destination is never replaced.
- Every new journal entry records the admitted parent identity. Windows opens a
  retained non-reparse parent handle, verifies that identity before opening the
  source relative to it, and uses the same handle as rename authority. Legacy
  journals without parent identity remain inspectable but cannot Recovery/Undo.
- Directory support does not combine selecting a directory entry with recursive
  discovery. Directory symlinks, junctions, and reparse points are not followed,
  moves remain same-parent, and initial plans containing both an ancestor and its
  descendant are blocked.
- Unsafe Rust is forbidden outside `renamewright-windows-native`. In that crate
  it is limited to the reviewed Win32 identity query, variable-length rename
  buffer construction, and handle-based rename calls. Every relevant audit must
  check current stable Rust, Microsoft bindings, OS facilities, and reviewed
  safe wrappers and remove or narrow the exception when equivalent safe,
  handle-preserving semantics are available. Source-level Semgrep suppressions
  are confined to those individually documented blocks; SARIF publication
  excludes only results marked `inSource` and preserves unsuppressed or
  externally suppressed findings.
- GitHub workflows use least privilege, immutable action pins, locked
  dependencies, and reviewed checksums for downloaded executables. Acceptance
  artifacts bind the source SHA, manifest, binaries, checklist, and path-free
  SBOM through SHA-256 evidence.

## Reportable findings and severity context

Report a finding when reachable product or build code violates an invariant and
causes or realistically enables native-path or journal disclosure, unintended
filesystem access or replacement, authorization bypass, journal corruption or
unrecoverable state, resource exhaustion, unsafe-boundary unsoundness, or
supply-chain/artifact substitution.

Local-only impact is not automatically informational. A reliable UI or
automation path disclosure or local application memory-exhaustion path can be
medium when it crosses an explicit boundary or materially affects availability.
Filesystem corruption, arbitrary overwrite, mutation without current native
confirmation, escape from selected parents, or exploitable memory unsafety may
justify high severity depending on prerequisites and blast radius. Critical
severity normally requires exceptional impact such as remotely reachable code
execution or broad release supply-chain compromise. Missing defense in depth
without a realistic source-to-sink path is generally low or informational.

Calibrate severity using demonstrated reachability, required local interaction,
whether the test-only automation feature is already required, filesystem scope,
recovery possibility, and whether impact is limited to one local application
instance.
Do not downgrade solely because the application is desktop-only, and do not
upgrade based on hypothetical internet or multi-user infrastructure that the
repository does not contain.

## Out of scope and non-findings

- Vulnerabilities in the original DarkNamer executable or unrelated software
  are outside this repository's scope.
- Claims that require a hosted service, account system, updater, remote content,
  shell plugin, or general-purpose filesystem API that is absent from the
  reviewed revision are not reachable product findings.
- Linux and macOS packaging defects are not release blockers while Windows
  x86-64 remains the only supported release target. Portable Rust-domain defects
  and cross-platform build or test-boundary failures remain in scope.
- Social engineering, physical-device access, or an attacker who already has
  equivalent arbitrary code execution as the same OS user is out of scope
  unless the application increases capability, persistence, disclosure, or
  destructive impact.
- The expected SmartScreen warning or absence of a signature on an explicitly
  labelled unsigned acceptance build is not by itself a vulnerability. A false
  signature claim, checksum mismatch, provenance failure, or unsigned artifact
  represented as a signed release is reportable.
- Displaying the selected entry's base name is intended product behavior. Native
  parent paths and journal-native paths are not intended UI or automation data.

These exclusions are not suppression authority for a concrete invariant
violation. Validate source, reachability, and impact for each candidate.

## Known limitations and accepted release risk

- Only the latest public native release and the current `master` line receive
  security fixes. Review the exact source SHA and artifact checksums.
- Public portable and acceptance artifacts are explicitly disclosed as unsigned.
  SignPath Foundation is the selected future signing path, subject to external
  project acceptance and trusted-build configuration. Never infer a signature
  from checksums alone.
- Hosted CI does not perform interactive packaged GUI testing, and ReFS coverage
  requires a compatible external Windows environment. Lack of ReFS evidence is
  recorded as unavailable rather than inferred from NTFS results.
- `FILE_ID_INFO` is a revalidation identity for an operation or journal recovery
  interval, not a permanent globally unique file UUID.
- Tests and scanner success are supporting evidence, not proof of FFI soundness,
  race freedom, correct recovery under every interruption, or artifact
  reproducibility.
