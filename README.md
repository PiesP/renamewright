# Renamewright

**Plan every rename.**

Renamewright is a Windows-first, local desktop workbench for building,
reviewing, executing, recovering, and undoing bulk file and directory rename
plans. The application is a native Rust executable built with `eframe`/`egui`;
it has no browser runtime, JavaScript toolchain, sidecar, account, telemetry, or
background network dependency.

## Status

The native workbench admits explicitly selected peer files and directories from
the picker or drag/drop, keeps native paths below the application boundary, and
renders a path-free plan projection. Its ordered rules, per-source overrides,
diagnostics, local presets, JSON/CSV inspection, and virtualized 10,000-entry
preview are implemented in Rust.

Apply requires the exact current unblocked plan, native confirmation, fresh
identity validation, a single mutation lock, and a new durable no-replace
journal. Ledger, Recovery, Rollback, Resume, Reconcile, cancellation, and Undo
reuse that journaled execution boundary. Explicit directory selection never
enumerates children; extension-only rules skip directories, ancestor/descendant
selections are blocked, reparse points are not followed, and renames remain
within the original parent.

The primary distributable is one portable `renamewright.exe`. Its Windows MSVC
runtime is statically linked, so the executable does not require a separately
installed Visual C++ runtime. Windows system libraries remain operating-system
dependencies. The default feature set contains no custom inspection listener or
fixture loader. Test automation is compiled only with the explicit `automation`
Cargo feature and still requires `--automation`, an absolute disposable
`--automation-root`, and a visible test-mode banner.

## Interaction design

The native UI follows a direct-command workbench model inspired by the short,
predictable workflow of classic batch renamers without reproducing their hidden
state or unsafe mutation behavior:

1. Add explicitly selected files or directory entries. Adding a directory entry
   never implies recursive discovery.
2. Press a labelled command such as Replace, Prefix, Suffix, Number, Remove
   range, Extension, or Case.
3. Enter only the values required by that command. The first field receives
   focus, the live preview updates while editing, `Enter` commits the rule, and
   `Escape` closes the edit without applying filesystem changes.
4. Review the compact ordered rule chain and the before-and-after name table.
5. Resolve every blocked row, then confirm the exact changed count shown by the
   Apply action.

Common rules are buttons rather than a rule-type dropdown. Only one inline rule
editor is open at a time, while committed rules remain visible and reorderable.
Each committed rule has a dedicated drag handle and visible insertion marker;
the earlier/later buttons and `Alt+Left`/`Alt+Right` remain equivalent keyboard
and accessibility paths. Dropping Explorer entries shows an admission count
before release and reports how many new entries were accepted.

Preview rows are selectable. A second click or `Enter` opens the existing
per-entry override editor, count chips move directly to the first matching row,
and a diagnostic links to the last changed rule only when the complete rule
trace supports that attribution. `/` focuses name search. The source/proposed
column divider is resizable and resettable as a persisted view preference.
`Remove from plan` unregisters the opaque source ID and replans without deleting
or renaming the filesystem entry.

Rename Ledger, Recovery, Undo, presets, and inspection exports are on-demand
surfaces so the preview retains the majority of the window. A disabled Apply
action always has persistent explanatory text; safety information never depends
on hover or colour alone.

Appearance keeps System, Light, and Dark in one short source-bar menu. Accent
presets, Standard or Compact density, and optional preview details remain behind
Advanced appearance. These settings persist locally as view preferences only;
they never change or authorize the rename plan, and Windows high contrast
continues to own application colours when active.

The design direction is an **instrumented direct-manipulation workbench**:
labelled commands and continuously visible names provide immediate proposal
feedback, required inputs appear progressively, and filesystem mutation remains
a separate inspect-and-commit step. It adapts the Windows
[command-bar](https://learn.microsoft.com/en-us/windows/apps/develop/ui/controls/command-bar),
[keyboard](https://learn.microsoft.com/en-us/windows/apps/develop/input/keyboard-interactions),
and
[confirm-or-undo](https://learn.microsoft.com/en-us/windows/apps/design/basics/commanding-basics)
principles to `eframe`/`egui` without pretending to be a WinUI application.
Future visual work should strengthen the single source → rule → preview → Apply
task spine, contextual diagnostics, and adaptive list/details layout rather than
introducing a wizard, ribbon, dashboard, or modal rule editor.

## Development

Install the stable Rust version pinned by `rust-toolchain.toml`, then use Cargo:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-targets --all-features --locked
cargo run --package renamewright-app --bin renamewright --locked
```

The native performance gate is also Rust-owned:

```bash
cargo run --release --locked --package renamewright-platform --example large_batch_budget
cargo run --release --locked --package renamewright-application --example service_batch_budget
```

Windows release and acceptance builds use the checked-in PowerShell packaging
scripts. They bind artifacts to the source SHA, reject automation markers in the
production executable, generate a Cargo-lockfile CycloneDX SBOM, and publish
SHA-256 manifests. See [RELEASE.md](RELEASE.md) and
[VALIDATION.md](VALIDATION.md) for the remaining external evidence boundaries.

## Product commitments

- No filesystem mutation until the user reviews an explicit plan and confirms
  the exact current operation.
- Deterministic, reorderable rules with immediate preview.
- Direct rule buttons with keyboard-focused, progressively disclosed inputs.
- Collision, invalid-name, stale-source, identity, and case-only diagnostics.
- Durable journaling, best-effort rollback, explicit Recovery, and revalidated
  Undo rather than impossible safety promises.
- Native path handling without lossy Unicode conversion or UI path disclosure.
- No telemetry, remote content, shell access, plugin system, or runtime updater.
- No custom automation listener in the default production build.

## Code signing policy

Current acceptance and portable candidate artifacts are explicitly unsigned.
Future signed releases may use SignPath.io with a SignPath Foundation
certificate after the project and trusted-build integration are approved. A
release must state its signing status and publish source-bound checksums; an
unsigned artifact must never be represented as signed.

## License

Renamewright is available under the [MIT License](LICENSE).
