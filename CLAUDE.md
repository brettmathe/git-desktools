# git-desktools

Lightweight cross-platform git GUI (Tauri 2 + Rust + React/TypeScript).

**Read `PLAN.md` first** — it has the full spec, decisions made with the user, current status,
backend/frontend design, and verification steps. UI reference screenshots are in `docs/reference/` (local only, gitignored: they contain private repo data).

- Package manager: pnpm. Dev: `pnpm tauri dev`. Typecheck: `pnpm typecheck`. Frontend tests: `pnpm test`. Rust tests: `cd src-tauri && cargo test`.
- Primary test repo (many branches): `~/wahoo/dex1/mfe/lobby-mfe` — read-only, never modify it.
- Keep it simple: no PRs/issues/teams/AI features. Exception: **Smart Environments / Smart Features**
  (multi-repo feature branches with parked stashes) is approved; see the Smart Features section of `PLAN.md`.
- Update the Status section of `PLAN.md` as work lands.

## Publishing a new version

Full steps and troubleshooting are in the README ("Publishing a new version"). In short, from a clean,
up-to-date `main` with the release's work merged:

```sh
node scripts/bump-version.mjs <x.y.z>     # package.json, tauri.conf.json, Cargo.toml, Cargo.lock
pnpm version:check
git commit -am "Release v<x.y.z>"
git tag v<x.y.z> && git push origin main v<x.y.z>
```

The tag triggers `.github/workflows/release.yml` (tests, universal macOS build, GitHub Release with
`.dmg`, signed `.app.tar.gz` + `.sig`, and `latest.json`, which installed apps poll). Watch it with
`gh run watch`, then confirm the four assets and that `releases/latest/download/latest.json` shows the
new version.

- Confirm with the user before pushing a release commit or tag: publishing ships to every install.
- Tag only after the work is merged to `main`; the tag must equal `tauri.conf.json`'s version or CI
  refuses it. Never reuse a published version; to retry a failed run, delete and re-push the tag.
- Signing key: `~/.tauri/git-desktools.key` (+ `.password`), mirrored in the repo secrets
  `TAURI_SIGNING_PRIVATE_KEY[_PASSWORD]`. Never commit or print it.
- Keep `tauri-plugin-updater` (crate) and `@tauri-apps/plugin-updater` (npm) on the same minor; the
  CLI refuses a mismatch. Don't bypass pnpm's `minimumReleaseAge` (no `pnpm-workspace.yaml` exclusions).

## Driving the UI in tests

The native window can't be screenshotted from this terminal (no Screen Recording permission), so UI
checks run the real frontend in headless Chrome against the real backend:

1. `pnpm dev` (or `pnpm tauri dev`) serves the UI on :1420. In a plain browser, `src/devBridge.ts`
   routes `invoke` to the test-only HTTP bridge in `src-tauri/src/dev_bridge.rs`.
2. Start the bridge:
   `GDT_BRIDGE_WRITE_ROOT=<scratch dir>/ GDT_BRIDGE_SETTINGS='{"openTabs":["<repo>"],"activeTab":"<repo>","autoFetchSeconds":0}' cargo test --release dev_bridge -- --ignored --nocapture`
3. Drive it with playwright-core using system Chrome (`executablePath: /Applications/Google Chrome.app/...`).

Safety: the bridge refuses stage/commit/fetch/pull/push/clone/init outside `GDT_BRIDGE_WRITE_ROOT`.
Only ever point write tests at throwaway repos in the scratchpad; scope selectors to the visible tab
(`.tab-pane:visible`), because hidden tabs stay mounted and have their own toolbar buttons.
