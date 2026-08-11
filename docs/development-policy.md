# Development policy

This document records the public engineering and GitHub policy for Renamewright.
Repository configuration, lockfiles, and workflow files are authoritative when
details drift.

## AI policy

AI assistance is allowed for bounded engineering work. Its output is treated as
untrusted draft material until a contributor reviews the diff and verifies the
behavior. Agents receive least authority: reading and local validation are the
default, while remote writes, publication, deployment, signing, secret access,
and external communication require explicit authorization.

Project data remains local by default. Filenames, native paths, file metadata,
journals, recovery records, and vulnerability details are not uploaded to AI or
other external services merely because a tool can accept them. Public source may
be sent to an explicitly requested review service; private data still requires a
separate decision.

Validation evidence must be literal. A skipped Windows check is reported as
skipped, a queued workflow is pending, and a scanner candidate is not a confirmed
vulnerability. AI review complements deterministic format, lint, type, test,
build, and security gates; it never replaces them.

Detailed agent and Copilot instructions are local, gitignored workspace aids.
They summarize current commands and boundaries but cannot override this policy,
tracked configuration, or contributor intent.

## Git policy

- `master` is the default branch.
- Maintainer work starts from an updated `master` on `codex/<topic>` and is
  integrated with `--no-ff`; hooks reject direct default-branch commits and
  non-merge default-branch pushes.
- External changes use pull requests with one approval, dismissal of stale
  approvals, last-push approval, resolved conversations, and strict required
  checks.
- Force pushes and branch deletion are disabled. Administrators may bypass the
  pull-request gate for the documented local merge workflow, but the resulting
  default-branch commit must still be a two-parent merge.
- Merge commits and squash merges are allowed; rebase merges are disabled.
  Branches are deleted after hosted merges and auto-merge may be enabled.

## GitHub Actions policy

Workflows receive read-only repository contents unless a job documents a narrower
write need. Third-party actions are pinned to full commit SHAs. Pull-request code
is never executed by a privileged `workflow_run` job, and secrets are not made
available to untrusted forks.

Fast CI covers formatting, linting, strict frontend types, Rust formatting and
Clippy, unit coverage, rendered Chromium behaviour, workspace tests, and a
Windows Tauri build. Security CI covers CodeQL for JavaScript/TypeScript, Rust,
and Actions plus OSV dependency advisories and Semgrep. Expensive mutation,
packaging, and signed-release work are added only when the corresponding
implementation exists.

OSV exceptions must identify one advisory, explain why the current dependency
graph cannot remove it, and expire within 90 days. An exception suppresses only
that known advisory; newly disclosed findings still fail the gate. Expired
entries are removed or renewed only after checking the current upstream graph
and documenting the new decision in review.

## Dependency policy

Dependabot checks npm, Cargo, the Rust toolchain, and GitHub Actions daily. Normal
updates observe a 24-hour cooling window. Patch and minor updates may be grouped;
major updates remain visible for manual review. pnpm rejects unreviewed dependency
build scripts and exotic transitive sources. Runtime dependencies are admitted
only for a concrete boundary and are reviewed for native code, build scripts,
maintenance, license, and data behavior.

## Release policy

No tag, installer, portable archive, code signature, or release is published by
an implementation task unless release publication was explicitly requested.
Release artifacts will eventually require checksums, an SBOM, Windows smoke
tests, and a user-selected signing method.
