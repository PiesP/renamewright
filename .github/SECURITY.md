# Security policy

Renamewright is pre-release software. Security support currently applies to the
latest commit on `master`; no stable version is supported yet.

## Reporting a vulnerability

Do not disclose vulnerabilities publicly. Prefer a private GitHub Security
Advisory for this repository. If that is unavailable, open an issue requesting a
private contact channel without including exploit details.

Useful reports include the affected operating system, exact build or commit,
impact, reproduction steps, relevant structured errors, and whether filesystem
state changed. Do not attach private filenames or file contents unless requested
through the private channel.

## Security and privacy model

- Rename planning and execution are local; no account, telemetry, cloud sync, or
  runtime network content is required.
- The WebView receives opaque source identifiers rather than arbitrary native
  write paths.
- New-plan Apply accepts only the exact current plan ID in the native shell and
  requires native confirmation, fresh identity validation, a backend-selected
  journal, and journaled no-replace execution. No path-bearing Apply command is
  registered.
- Recovery and Undo of existing journals require a current path-free inspection,
  native confirmation, fresh identity validation, and journaled no-replace
  execution.
- Runtime code is bundled. Dynamic code execution, shell commands, plugins, and
  user scripts are out of scope.

## Development security

GitHub Actions use read-only default permissions and immutable action pins.
Dependabot monitors npm, Cargo, the Rust toolchain, and Actions after a 24-hour
cooling period. CodeQL, dependency advisory and policy checks, strict compilers,
tests, platform builds, checksums, and a source-bound SBOM provide independent
evidence. AI or scanner findings require human validation before remediation or
severity claims.
