# Changelog

This file summarizes user-visible changes. Source history and each GitHub
Release retain the complete commit list and downloadable evidence.

## 0.2.0 - 2026-08-15

The first public native Rust release replaces the historical Tauri bootstrap
with one portable Windows x86-64 executable.

### Added

- Preview-first file and directory rename planning with direct Replace, Prefix,
  Suffix, Number, Remove range, Extension, and Case commands.
- Reorderable rules, per-entry overrides, searchable virtualized previews,
  local presets, JSON/CSV inspection, and appearance preferences.
- Confirmation-gated Apply with durable journals, Recovery, Rollback, Resume,
  Reconcile, cancellation, and revalidated Undo.
- Source-bound release manifests, SHA-256 checksums, and a CycloneDX SBOM.

### Fixed

- Restored missing visible text by retaining a compact base font and loading
  Korean and emoji fallbacks only when required.
- Made the full rule card a usable drag target while preserving buttons,
  keyboard commands, accessibility actions, and visible insertion feedback.
- Hardened rename identity, collision, reserved-name, journal, and concurrent
  mutation boundaries.

### Performance

- Reduced the portable executable footprint and kept UI-owned code optimized
  for runtime performance.
- Deferred and cached preview, ledger, source-admission, font, and document work
  to keep large plans responsive and memory-bounded.

### Distribution

- The supported download path is the latest GitHub Release.
- The portable executable is unsigned and may trigger SmartScreen. Verify
  `SHA256SUMS.txt` before running it.
- Renamewright has no runtime updater; close the app and replace the executable
  to update. Presets and journals remain in the user's data directory.

## 0.1.0 - 2026-08-12

Unsupported historical Tauri-based bootstrap release. It is retained for
provenance and is not the native application described by current documentation.
