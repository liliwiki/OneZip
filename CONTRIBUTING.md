# Contributing

Contributions are welcome when they make extraction safer, add a well-supported format, or improve how OneZip fits into Windows, without weakening the guarantees around deleting archives in place.

Before submitting a change:

1. Keep the engine in `onezip-core` free of user-interface code; the window, the command line, and the context menu are thin front ends over it.
2. Never mark data as done before it is on disk. Anything that deletes or truncates an archive must survive a power cut or a crash at any point.
3. Add a preview or explicit confirmation before destructive operations started from the window or the command line.
4. Never commit credentials, signing certificates or keys, personal paths, build output, or unrelated binaries.
5. Changes to `third_party/` must be listed in that crate's `ONEZIP-CHANGES.md` and in `THIRD_PARTY_NOTICES.md`.
6. Record every new third-party component, exact version, source, and license in `THIRD_PARTY_NOTICES.md`.
7. Run `cargo test --workspace` and `cargo clippy --workspace --all-targets`. For context menu or window changes, also run `tools\uitest\regress.ps1` with OneZip installed.
8. Explain which files changed, what user-facing behavior changed, and how the result was verified.

## Commit Convention

Use Conventional Commits with a concise English description:

```text
type(scope): short description
```

Common types are `feat`, `fix`, `perf`, `docs`, `chore`, `refactor`, `test`, `build`, and `ci`.

| Scope | Use for |
|---|---|
| `core` | Engine: extraction, compression, checksums |
| `cli` | `onezipc.exe` |
| `app` | `OneZip.exe` window, progress windows, dialogs |
| `shell` | Context menu DLL and the classic registry menu |
| `packaging` | Sparse package, signing, install and uninstall scripts |
| `third-party` | Modified copies under `third_party/` |
| `readme` | English or Chinese project overview |
| `project` | Repository policy, templates, and licensing |
| `release` | Version metadata, changelog, and release notes |

Examples:

```text
feat(core): extract split 7z archives
fix(shell): hide extraction items for folders
perf(third-party): flush extracted files in parallel on Windows
docs(readme): update bilingual project overview
```

Keep each commit focused on one logical change.
