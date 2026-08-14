# Validation evidence boundary

Automated repository gates cover Rust formatting, strict Clippy, default and
automation feature tests, 10,000-entry planning budgets, Windows compilation and
runtime startup where the hosted renderer is available, production-listener
exclusion, source binding, Cargo policy, OSV, CodeQL, Semgrep, checksums, and the
CycloneDX SBOM.

The following checks require an interactive or specifically provisioned Windows
environment and are not implied by ordinary CI success:

- native file and folder pickers and real Explorer drag/drop;
- Korean IME composition and keyboard-only focus traversal;
- AccessKit/Windows UI Automation against the packaged executable;
- 100, 125, 150, 200, and 250 percent DPI plus active high contrast;
- NTFS collision, case-only, swap, cycle, interrupted Apply, Recovery, Rollback,
  Resume, Reconcile, cancellation, and Undo flows;
- ReFS behavior, code signing, SmartScreen reputation, and signed-artifact
  provenance.

For the Stage 6H cutover these manual checks are intentionally skipped. A release
must bind any later evidence to the exact source SHA and artifact digest, keep
untested configurations explicit, and never infer ReFS, signing, or full visual
matrix coverage from hosted Windows tests.
