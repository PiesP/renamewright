# Portable Windows release process

The canonical user distribution path is the repository's
[GitHub Releases page](https://github.com/PiesP/renamewright/releases). A tag
named `v<workspace-version>` publishes one Windows x86-64 portable bundle. A
manual `workflow_dispatch` run creates only a 14-day release-candidate artifact;
it never creates or changes a public release.

Version `0.2.0` is the first public release of the native Rust application.
Version `0.1.0` remains available as unsupported historical evidence for the
previous Tauri-based bootstrap and must not be replaced or retagged.

## Artifact boundary

The portable executable is built from the default Cargo feature set. The
`automation` feature, inspection probe, test fixture loader, and loopback
listener are not release inputs. Every published bundle contains:

- `Renamewright-<version>-windows-x86_64-portable.exe`;
- `Renamewright-<version>.cdx.json`;
- `release-manifest.json`;
- `README.txt`; and
- `SHA256SUMS.txt`.

The executable is unsigned. A release description must say so explicitly, and
checksums must not be described as a code-signing signature.

## Automated publication

The `Portable Release` workflow:

1. tests the default Rust feature set;
2. builds only `renamewright-app`'s `renamewright` binary in release mode;
3. rejects known automation markers in that binary;
4. packages the versioned executable with a source-SHA-bound manifest;
5. generates a CycloneDX SBOM from `Cargo.lock` with a checksum-verified Syft
   binary;
6. uploads the complete bundle as an immutable GitHub Actions artifact and
   records its artifact digest; and
7. on tag runs only, downloads that exact artifact in a separately permissioned
   job, verifies `SHA256SUMS.txt`, and publishes it as the latest GitHub Release.

Repository-wide workflow permissions remain read-only. Only the tag-gated
publication job receives `contents: write`, and it does not rebuild or modify
the Windows bundle.

## Release checklist

1. Update the workspace version and user-facing changelog on a work branch.
2. Complete the repository format, strict Clippy, locked test, performance, and
   Windows packaging gates for the final `master` merge SHA.
3. Create the signed annotated tag `v<workspace-version>` at that exact merge
   SHA and push the tag once.
4. Wait for both the Windows packaging job and the tag-gated publication job.
5. Confirm that the public release targets the expected tag and source SHA,
   contains all five assets, is marked latest, and states that it is unsigned.
6. Download the published bundle into a fresh directory and verify every entry
   in `SHA256SUMS.txt` before announcing the release.

Removing or replacing the portable executable does not remove user presets or
journals. Close the application before backing up or restoring its data root,
and resolve or deliberately archive incomplete journals before deleting it.
