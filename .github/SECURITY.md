# Security policy

Renamewright is pre-release software. Security support currently applies to the
latest commit on `master`; no stable version is supported yet.

## Reporting a vulnerability

Do not disclose vulnerabilities publicly. Prefer a private GitHub Security
Advisory for this repository. If that is unavailable, open an issue requesting a
private contact channel without including exploit details.

Useful reports include the operating system, exact source and artifact SHA,
impact, reproduction steps, structured errors, and whether filesystem state
changed. Do not attach private filenames or file contents unless requested
through the private channel.

## Security and privacy model

- Planning and execution are local; no account, telemetry, cloud sync, updater,
  or runtime network content is required.
- Native paths stay in Rust-owned services. UI, automation, exports, and ordinary
  logs receive opaque IDs, display names, and structured codes.
- Apply, Recovery, and Undo require native confirmation, fresh identity
  validation, a backend-selected journal, and journaled no-replace execution.
- The production feature set contains no custom listener or fixture API.
- Dynamic code execution, shell commands, plugins, and user scripts are absent.

## Development security

GitHub Actions use read-only default permissions and immutable action pins.
Dependabot monitors Cargo, the Rust toolchain, and Actions after a 24-hour
cooling period. CodeQL, OSV, Cargo policy, Semgrep, strict compilers, tests,
Windows builds, checksums, production automation-marker rejection, and a
source-bound Cargo SBOM provide independent evidence. Scanner findings still
require source-to-sink validation before remediation or severity claims.
