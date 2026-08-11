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

The repository contains the approved planning baseline and a working read-only
Rust/Tauri/Solid vertical slice. The desktop picker admits files into a
Rust-owned source registry, the Workbench previews a prefix rule, and Windows
name diagnostics block invalid proposals. Filesystem mutation is not exposed to
the application.

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
checks plus Rustfmt, Clippy, and all Rust workspace tests. See
[the delivery plan](docs/delivery-plan.md) for the implementation sequence.
`pnpm verify:full` additionally exercises the rendered flow and responsive
layouts in Chromium.

## Product commitments

- No file changes until the user reviews an explicit plan.
- Deterministic, reorderable rules with immediate preview.
- Collision, invalid-name, stale-source, and case-only rename diagnostics.
- Crash-aware execution with a durable journal and best-effort rollback.
- Undo that revalidates the filesystem instead of promising impossible safety.
- Native path handling without lossy Unicode conversion.
- No telemetry, remote content, shell access, or background network traffic in
  the first release.

Read the [product design](docs/product-design.md),
[architecture](docs/architecture.md), and
[delivery plan](docs/delivery-plan.md) before implementation.
