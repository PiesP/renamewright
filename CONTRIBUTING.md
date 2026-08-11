# Contributing to Renamewright

Renamewright accepts carefully reviewed human and AI-assisted contributions.
The contributor remains responsible for every submitted line, claim, and
external action regardless of which tools helped produce it.

## Development workflow

1. Start from the current `master` branch and create a focused work branch.
2. Keep commits conventional, independently reviewable, and buildable when
   practical.
3. Run the narrowest relevant check first, then `pnpm verify` before publishing
   a substantive change.
4. Use a pull request for external contributions. Maintainer integrations use
   an explicit merge commit so the reviewed branch remains visible in history.

Source, comments, documentation, commit messages, and pull requests are written
in English. User-facing strings belong in message catalogs once localization is
introduced.

## AI-assisted work

AI tools may help with research, implementation, tests, documentation, and
review. They do not reduce the contributor's obligations:

- review the complete diff and understand security-sensitive behavior;
- never provide secrets, private user paths, file contents, signing material,
  or unpublished vulnerability details to an external model without explicit
  authorization;
- do not claim a test, review, scan, browser run, or platform check that did not
  actually complete;
- treat generated security findings as candidates until source-to-sink impact
  and reachability are validated;
- do not let an agent publish, deploy, sign, change repository settings, or
  contact another person unless that external action was explicitly requested;
- preserve unrelated work and keep generated artifacts out of commits unless
  they are deliberate, reproducible project outputs.

No special AI attribution is required. The Git author and pull-request author
are the accountable contributors.

## Safety boundary

The first product milestone is read-only. It may admit filenames, create an
in-memory rename proposal, and export an inspectable plan, but it must not rename,
move, replace, or delete filesystem entries. New native capabilities require a
threat-boundary review and tests before they are exposed to the WebView.

## Reporting problems

Use ordinary issues for non-sensitive bugs. Follow
[the security policy](.github/SECURITY.md) for vulnerabilities or privacy issues.
