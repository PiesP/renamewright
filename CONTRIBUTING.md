# Contributing to Renamewright

Renamewright accepts carefully reviewed human and AI-assisted contributions.
The contributor remains responsible for every submitted line, claim, and
external action regardless of which tools helped produce it.

## Development workflow

1. Start from the current `master` branch and create a focused work branch.
2. Keep commits conventional, independently reviewable, and buildable when
   practical.
3. Run the narrowest relevant Cargo check first, then Rustfmt, strict Clippy,
   and the complete workspace test suite before publishing substantive work.
4. Use a pull request for external contributions. Maintainer integrations use
   an explicit merge commit so the reviewed branch remains visible in history.

Optional Codex Security checks are documented in
[the security policy](.github/SECURITY.md). They are advisory, keep private
results outside the checkout, and do not replace deterministic Cargo, CodeQL,
OSV, Semgrep, Windows, or packaged-runtime validation.

Source, comments, documentation, commit messages, and pull requests are written
in English. User-facing strings must keep the native Korean/English catalog
complete.

## AI-assisted work

AI tools may help with research, implementation, tests, documentation, and
review. They do not reduce the contributor's obligations:

- review the complete diff and understand security-sensitive behavior;
- never provide secrets, private paths, file contents, signing material, or
  unpublished vulnerability details to an external model without approval;
- never claim a test, scan, platform check, or visual review that did not run;
- validate generated security findings before remediation or severity claims;
- do not let an agent publish, sign, change repository settings, or contact
  another person unless that external action was explicitly requested; and
- preserve unrelated work and commit only deliberate reproducible artifacts.

## Safety boundary

All rename mutations require the application-service plan, native confirmation,
fresh filesystem identity checks, a durable journal, and no-replace execution.
Changes to admission, planning, confirmation, Windows handles, journals,
Recovery, or Undo require a threat-boundary review and focused regression tests.

## Reporting problems

Use ordinary issues for non-sensitive bugs. Follow
[the security policy](.github/SECURITY.md) for vulnerabilities or privacy issues.
