# Security policy

Security fixes are provided for the latest public native release and the current
development line. Historical bootstrap builds do not receive fixes.

| Version | Supported |
| --- | --- |
| `0.2.x` | Yes |
| `master` | Best effort before the next release |
| `0.1.x` and older | No |

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
- The production feature set contains no custom listener or automation profile API.
- Dynamic code execution, shell commands, plugins, and user scripts are absent.

## Development security

GitHub Actions use read-only default permissions and immutable action pins.
Dependabot monitors Cargo, the Rust toolchain, and Actions after a 24-hour
cooling period. CodeQL, OSV, Cargo policy, Semgrep, strict compilers, tests,
Windows builds, checksums, production automation-marker rejection, and a
source-bound Cargo SBOM provide independent evidence. Scanner findings still
require source-to-sink validation before remediation or severity claims.

Codex Security is an advisory development scanner rather than a product runtime
dependency. The integrity-locked CLI is installed into a private directory
outside the checkout and uses Renamewright-specific threat-model and scan
instructions. Local reports and state also stay outside the repository because
they can contain filenames, source excerpts, and vulnerability details.

Use the following report-only local checks after signing in with Codex Security:

```bash
scripts/security/codex-security.sh dry-run
scripts/security/codex-security.sh working-tree
scripts/security/codex-security.sh branch origin/master
scripts/security/codex-security.sh full
```

Review and reconcile saved findings through the same locked CLI and private
state directory:

```bash
scripts/security/codex-security.sh login --device-auth
scripts/security/codex-security.sh login-status
scripts/security/codex-security.sh scans list
scripts/security/codex-security.sh scans rerun PREVIOUS_SCAN_ID
scripts/security/codex-security.sh scans compare PREVIOUS_SCAN_ID CURRENT_SCAN_ID
scripts/security/codex-security.sh findings list
scripts/security/codex-security.sh validate findings.json "Recheck the reviewed finding"
scripts/security/codex-security.sh findings false-positive OCCURRENCE_ID \
  --reason "Specific reviewed evidence that makes this path unreachable"
scripts/security/codex-security.sh logout
```

Only use `false-positive` after source-to-sink review. A resolved comparison
requires the later scan to cover the original target and location without gaps;
otherwise the result remains unknown. Use `validate` for security-critical fixes
because scan comparison tracks result changes but does not prove remediation.

To opt into the high-severity pre-commit check without replacing the versioned
Git hook, run `git config hooks.codexSecurity true`. Disable it with
`git config --unset hooks.codexSecurity`. Authentication defaults to the stored
ChatGPT sign-in; output, state, maximum cost, authentication, and hook severity
can be changed through the environment variables listed by the helper's usage.

The GitHub Actions workflow remains disabled until the repository variable
`CODEX_SECURITY_ENABLED` is set to `true` and the `CODEX_SECURITY_API_KEY`
Actions secret is configured. It scans same-repository pull requests, supports
a manually requested full scan, uploads SARIF when available, and retains only
the manifest and coverage metadata for seven days. A missing finding is not
proof of remediation: compare a complete follow-up scan with the original and
validate the source change before resolving or dismissing a report.
