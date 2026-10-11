# git-desktools

A lightweight, cross-platform git GUI — without the bells and whistles.
Built with Tauri 2 (Rust) and React.

- Tabs: one per repository, plus a Launchpad for Browse / Clone / Init and recent repos
- Commit graph with coloured lanes, merge dots, branch/tag pills and author avatars
- Commit details: message, author/committer, co-authors, changed files (path or tree), diffs
- Working-tree changes (`// WIP`): stage/unstage, commit, amend
- Fetch, Pull (fast-forward only), Push (sets upstream for new branches), background auto-fetch
- Docked terminal per repo that starts `claude` in the repo root (plus plain shells)

## Develop

Requires Rust (stable), Node 20+, pnpm and git.

```sh
pnpm install
pnpm tauri dev        # run the app
pnpm typecheck        # frontend types
cd src-tauri && cargo test
pnpm tauri build      # release bundle for the current OS
```

Settings live in the OS app-config directory (`settings.json`, e.g.
`~/Library/Application Support/com.brettmathe.gitdesktools/` on macOS). Use the gear icon to change the
auto-fetch interval.

See `PLAN.md` for the full spec and design notes.
