# Summary

Explain what changed, why it changed, and which trust or platform boundary is
affected.

## How to test

List the focused and publication-level checks that actually completed.

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-targets --all-features --locked
```

## Checklist

- [ ] Source, comments, documentation, and commits are in English
- [ ] Native paths stay below the application boundary and UI projections remain path-free
- [ ] Apply, Recovery, and Undo retain confirmation, revalidation, mutation-lock, journal, and no-replace guarantees
- [ ] Production default features contain no automation listener or fixture loader
- [ ] Rust errors remain typed and user-visible where relevant
- [ ] Relevant Linux, Windows, unit, native UI, performance, or packaged checks completed or are explicitly noted as unavailable
- [ ] User-visible behavior and security documentation were updated where needed
- [ ] AI-assisted claims and scanner findings were independently verified
