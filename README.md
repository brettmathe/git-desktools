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

## Publishing a new version

Releases are built by GitHub Actions when a `v*` tag is pushed. Installed apps poll
`releases/latest/download/latest.json` and offer the update in the tab bar.

1. Start from an up-to-date, clean `main` with everything for the release merged:

   ```sh
   git checkout main && git pull --ff-only && git status
   ```

2. Bump the version (semver, higher than the last release). This sets `package.json`,
   `src-tauri/tauri.conf.json` (the version the updater compares), `src-tauri/Cargo.toml` and
   `src-tauri/Cargo.lock`:

   ```sh
   node scripts/bump-version.mjs 0.2.0
   pnpm version:check                      # all four sites agree
   ```

3. Commit, tag and push both:

   ```sh
   git commit -am "Release v0.2.0"
   git tag v0.2.0
   git push origin main v0.2.0
   ```

4. Watch the build (about 10–20 minutes):

   ```sh
   gh run watch -R brettmathe/git-desktools $(gh run list -R brettmathe/git-desktools --workflow release.yml -L 1 --json databaseId --jq '.[0].databaseId')
   ```

   It runs the typecheck and tests, builds a universal macOS app, and publishes a GitHub Release.

5. Check the release has `git-desktools_<ver>_universal.dmg`, `…app.tar.gz`, `…app.tar.gz.sig` and
   `latest.json`, and that the manifest advertises the new version:

   ```sh
   gh release view v0.2.0 -R brettmathe/git-desktools
   curl -sL https://github.com/brettmathe/git-desktools/releases/latest/download/latest.json | head -c 300
   ```

Installed apps pick it up on their next check (launch, every few hours, or Settings → Check for
updates).

**If it goes wrong**

- *Tag doesn't match the version*: the workflow refuses to build. Delete the tag
  (`git tag -d v0.2.0 && git push origin :refs/tags/v0.2.0`), fix the version, and tag again.
- *Build or tests fail*: nothing is published. Fix on `main`, delete the tag as above, re-tag the
  fixed commit and push the tag again. Never re-use a version number that was already published.
- *Push of `.github/workflows/*` rejected*: the GitHub token needs the `workflow` scope.

**Prerequisites (one-time, already done)**: repo secrets `TAURI_SIGNING_PRIVATE_KEY` and
`TAURI_SIGNING_PRIVATE_KEY_PASSWORD`, from `~/.tauri/git-desktools.key` and
`~/.tauri/git-desktools.key.password`. That key signs every update and its public half is built into
the app: lose it and installed copies can no longer accept updates, so keep a backup and never commit
it. Local `pnpm tauri build` needs the same two values as environment variables.

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
