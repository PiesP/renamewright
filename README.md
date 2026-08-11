# DarkNamer Next

DarkNamer Next is a planned Windows-first desktop application for building,
reviewing, and safely executing bulk file rename plans.

This is a redesign, not a source or bug-compatible reconstruction of the legacy
`DarkNamer.exe`. The old executable is used only as evidence of useful jobs; the
new product owns its interaction model, safety guarantees, and implementation.

## Status

The repository currently contains the approved planning baseline. Production
code and dependency manifests will be introduced through the vertical slices in
[the delivery plan](docs/delivery-plan.md).

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
