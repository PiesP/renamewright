# Renamewright

**Plan every rename.**

Renamewright is a planned Windows-first desktop application for building,
reviewing, and safely executing bulk file rename plans. The public product,
repository, and executable share the `Renamewright` name; machine-facing
identifiers use `renamewright`.

This is a redesign, not a source or bug-compatible reconstruction of the legacy
`DarkNamer.exe`. The old executable is used only as evidence of useful jobs; the
new product owns its interaction model, safety guarantees, and implementation.

## Status

The repository contains the complete read-only planning milestone. Native picker
and drag/drop admission retain paths in Rust, the Workbench virtualises and
filters large prefix plans, and Windows name, occupied-destination, and stale
source diagnostics block unsafe proposals. Plans can be inspected as versioned,
path-free JSON and exported through a native create-new dialog. Source filesystem
mutation is not exposed to the application.

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
- No telemetry, remote content, shell access, or background network traffic in
  the first release.
