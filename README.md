# Renamewright

**Plan every rename.**

Renamewright is a Windows-first desktop application for building, reviewing,
and safely executing bulk file and directory rename plans. The public product,
repository, and executable share the `Renamewright` name; machine-facing
identifiers use `renamewright`.

This is a redesign, not a source or bug-compatible reconstruction of the legacy
`DarkNamer.exe`. The old executable is used only as evidence of useful jobs; the
new product owns its interaction model, safety guarantees, and implementation.

## Status

The repository contains the published 0.1.0 Windows baseline. Native picker and
drag/drop admission retain paths in Rust, the Workbench virtualises and filters
large rule-based plans, and Windows name, occupied-destination, and stale-source
diagnostics block unsafe proposals. Plans can be inspected as versioned,
path-free JSON and exported through a native create-new dialog. Applying a new
plan remains disabled; recovery and Undo are limited to journaled transactions,
fresh filesystem revalidation, and native user confirmation.

Post-0.1 development is replacing the Tauri/Solid shell with an
`eframe`/`egui` native Rust UI. The UI-independent
`renamewright-application` service now owns session state, source admission,
planning, inspection/export, Ledger, Recovery, Undo, cancellation, and execution
preparation; Tauri retains only its framework adapters and native dialogs. The
target artifact is one portable Windows executable with no Node.js, WebView2,
or sidecar runtime. New-plan Apply and directory execution remain disabled until
their dedicated journal, Windows, automation, and packaged UI gates pass.

## Development

Use the Rust, Node.js, and pnpm versions pinned by `rust-toolchain.toml` and
`package.json`. After installing the platform prerequisites for Tauri:

```bash
pnpm install --frozen-lockfile
pnpm verify
pnpm verify:full
pnpm tauri dev
```

`pnpm verify` runs frontend formatting, linting, strict types, coverage and build
checks plus Rustfmt, Clippy, and all Rust workspace tests. `pnpm verify:full`
additionally exercises the rendered flow and responsive layouts in Chromium.

## Product commitments

- No file changes until the user reviews an explicit plan.
- Deterministic, reorderable rules with immediate preview.
- Collision, invalid-name, stale-source, and case-only rename diagnostics.
- Crash-aware execution with a durable journal and best-effort rollback.
- Undo that revalidates the filesystem instead of promising impossible safety.
- Native path handling without lossy Unicode conversion.
- Explicitly selected file and directory entries renamed in place, without
  following directory symlinks or silently expanding a folder selection.
- No telemetry, remote content, shell access, or background network traffic in
  the first release.
- No custom automation listener in production builds; agent inspection is
  compiled only into an explicit, root-confined test artifact.

## Code signing policy

Renamewright 0.1.0 is an explicitly identified unsigned bootstrap release.
Future Windows releases are intended to use free code signing provided by
[SignPath.io](https://signpath.io/), certificate by
[SignPath Foundation](https://signpath.org/), after the project is accepted and
the trusted-build integration is configured.

- Committer, reviewer, and signing approver: [PiesP](https://github.com/PiesP).
- Only artifacts built from this public repository by the configured GitHub
  Actions release workflow may be submitted for signing.
- This program does not transfer information to other networked systems unless
  specifically requested by the user or the person installing or operating it.
  GitHub and SignPath apply their own privacy policies when a person visits
  their services or downloads a release.
- A release page must state whether its artifacts are signed and publish
  source-bound checksums. An unsigned artifact must never be represented as
  signed.

## License

Renamewright is available under the [MIT License](LICENSE).
