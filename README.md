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

The primary distributable is one portable `renamewright.exe`. The default
feature set contains no custom inspection listener or fixture loader. Test
automation is compiled only with the explicit `automation` Cargo feature and
still requires `--automation`, an absolute disposable `--automation-root`, and
a visible test-mode banner.

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
