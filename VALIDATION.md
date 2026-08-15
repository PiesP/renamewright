# Validation evidence boundary

Automated repository gates cover Rust formatting, strict Clippy, default and
automation feature tests, 10,000-entry planning budgets, Windows compilation and
runtime startup where the hosted renderer is available, production-listener
exclusion, source binding, Cargo policy, OSV, CodeQL, Semgrep, checksums, and the
CycloneDX SBOM.

Native UI tests additionally exercise stable-ID rule drag/drop with visible
insertion positions, keyboard reordering, source-drop feedback, selectable
preview rows, second-click and Enter override activation, count-chip navigation,
conservative rule-trace links, persisted/resettable column widths, and path-free
source exclusion that leaves the underlying entry unchanged. These tests use
egui's input and accessibility model; they do not substitute for packaged
Windows pointer, keyboard, or assistive-technology acceptance.

The following checks require an interactive or specifically provisioned Windows
environment and are not implied by ordinary CI success:

- native file and folder pickers and real Explorer drag/drop;
- packaged pointer rule reordering and column resizing, including cancel and
  boundary drops;
- Korean IME composition and keyboard-only focus traversal;
- AccessKit/Windows UI Automation against the packaged executable;
- 100, 125, 150, 200, and 250 percent DPI plus active high contrast;
- NTFS collision, case-only, swap, cycle, interrupted Apply, Recovery, Rollback,
  Resume, Reconcile, cancellation, and Undo flows;
- ReFS behavior, code signing, SmartScreen reputation, and signed-artifact
  provenance.

The native cutover did not complete this full external manual matrix. Publishing
version `0.2.0` adds source-bound packaging, checksum, SBOM, and public-release
verification; it does not close or imply the interactive gaps above. Any later
evidence must bind to the exact source SHA and artifact digest, keep untested
configurations explicit, and never infer ReFS, signing, or full visual matrix
coverage from hosted Windows tests.
