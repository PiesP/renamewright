# Portable Windows release process

The release candidate is a single `x86_64-pc-windows-msvc` executable built from
the default Cargo feature set. The `automation` feature, inspection probe, test
fixture loader, and loopback listener are not release inputs.

The `Portable Release` workflow:

1. tests the default Rust feature set;
2. builds only `renamewright-app`'s `renamewright` binary in release mode;
3. rejects known automation markers in that binary;
4. packages the versioned portable executable with a source-SHA-bound manifest;
5. generates a CycloneDX SBOM from `Cargo.lock` with a checksum-verified Syft
   binary; and
6. publishes `SHA256SUMS.txt` and an immutable GitHub Actions artifact digest.

The workflow deliberately does not create a public GitHub Release or claim a
signature. Public publication and SignPath submission require separate owner
authorization, successful source-bound Windows acceptance, and an explicit
statement of signing status.

Removing or replacing the portable executable does not remove user presets or
journals. Close the application before backing up or restoring its data root,
and resolve or deliberately archive incomplete journals before deleting it.
