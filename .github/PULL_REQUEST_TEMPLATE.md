## Summary

Describe what changed and why, and which part of OneZip it affects (engine, window, context menu, command line, packaging).

## Verification

- [ ] `cargo test --workspace` passes.
- [ ] `cargo clippy --workspace --all-targets` reports no warnings.
- [ ] Context menu or window changes were checked with `tools\uitest\regress.ps1`.
- [ ] Operations that delete or truncate archives stay safe after a crash or power cut at any point.
- [ ] Changes under `third_party/` are listed in that crate's `ONEZIP-CHANGES.md`.
- [ ] No credentials, signing keys, personal paths, or build output were added.
- [ ] English and Simplified Chinese documentation were updated where required.

## Third-party material

List every new external source, exact version, and license. Write `None` when this pull request adds no third-party material.
