# git-desktools

A lightweight, cross-platform git GUI — without the bells and whistles.
Built with Tauri 2 (Rust) and React.

- Tabs: one per repository, plus a Launchpad for Browse / Clone / Init and recent repos
- Commit graph with coloured lanes, merge dots, branch/tag pills and author avatars
- Commit details: message, author/committer, co-authors, changed files (path or tree), diffs
- Working-tree changes (`// WIP`): stage/unstage, commit, amend
- Fetch, Pull (fast-forward only), Push (sets upstream for new branches), background auto-fetch
- Docked terminal per repo that starts `claude` in the repo root (plus plain shells)
- Updates itself: checks GitHub Releases on launch and every few hours, installs on your say-so

## Install (macOS)

Universal build (Apple Silicon and Intel). The app is **not signed with an Apple Developer ID**, so how
you download it decides whether Gatekeeper gets involved.

With the GitHub CLI (no Gatekeeper prompt: CLI downloads never get the quarantine flag):

```sh
cd ~/Downloads
gh release download --repo brettmathe/git-desktools --pattern "*.dmg" --clobber
hdiutil attach git-desktools_*_universal.dmg
cp -R /Volumes/git-desktools/git-desktools.app /Applications/
hdiutil detach /Volumes/git-desktools
open /Applications/git-desktools.app
```

From the browser: download the `.dmg` from
[Releases](https://github.com/brettmathe/git-desktools/releases/latest), drag the app to Applications,
then clear the quarantine flag before the first launch (otherwise macOS says the app "is damaged"):

```sh
xattr -dr com.apple.quarantine /Applications/git-desktools.app
```

This applies to the first install only. After that, updates are signed with the app's own minisign key
and install from the update pill in the tab bar (or Settings → Updates) without prompting.

## Release

```sh
node scripts/bump-version.mjs 0.2.0   # package.json, tauri.conf.json, Cargo.toml, Cargo.lock
git commit -am "Release v0.2.0"
git tag v0.2.0 && git push origin main v0.2.0
```

The tag runs `.github/workflows/release.yml`: tests, a universal macOS build, and a GitHub Release with
the `.dmg`, the signed `.app.tar.gz` and `latest.json`, which installed apps poll. The workflow needs
the repo secrets `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`. The private key
lives outside the repo (`~/.tauri/git-desktools.key`); lose it and installed apps can no longer verify
updates, so keep a backup.

## Develop

Requires Rust (stable), Node 20+, pnpm and git.

```sh
pnpm install
pnpm tauri dev        # run the app
pnpm typecheck        # frontend types
cd src-tauri && cargo test
pnpm tauri build      # release bundle for the current OS (needs TAURI_SIGNING_PRIVATE_KEY[_PASSWORD])
```

Settings live in the OS app-config directory (`settings.json`, e.g.
`~/Library/Application Support/com.brettmathe.gitdesktools/` on macOS). Use the gear icon to change the
auto-fetch interval.

See `PLAN.md` for the full spec and design notes.
