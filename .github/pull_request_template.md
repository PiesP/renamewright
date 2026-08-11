# Summary

Explain what changed, why it changed, and which trust or platform boundary is
affected.

## How to test

List the focused and publication-level checks that actually completed.

```bash
pnpm verify
```

## Checklist

- [ ] Source, comments, documentation, and commits are in English
- [ ] The WebView receives no broader native capability than required
- [ ] The read-only milestone still cannot rename, move, overwrite, or delete files
- [ ] Rust and TypeScript errors remain typed and user-visible where relevant
- [ ] Relevant Linux, Windows, unit, browser, or packaged checks completed or are explicitly noted as unavailable
- [ ] User-visible behavior and security documentation were updated where needed
- [ ] AI-assisted claims and scanner findings were independently verified
