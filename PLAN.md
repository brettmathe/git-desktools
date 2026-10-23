# git-desktools — build plan

A lightweight, cross-platform git GUI: open local repos in tabs, browse the
commit graph, inspect commits/files/diffs, see working-tree changes (WIP), clone from GitHub,
and open a Claude terminal in the active repo. **No bells and whistles** — no PRs, issues,
teams, cloud patches, agents, AI recompose, profiles. The one approved larger feature is **Smart
Features** (see its section below).

UI reference screenshots live in `docs/reference/` (local only, gitignored) — look at
them before building UI:

| File | Shows |
|---|---|
| `01-commit-graph-small-repo.png` | Main repo view: tab bar, toolbar, sidebar, graph, commit detail panel, message tooltip |
| `02-sidebar-local-remote.png` | Sidebar LOCAL/REMOTE trees, HEAD branch highlighted green with ✓ and `1↓` |
| `03-launchpad-repo-management.png` | Launchpad tab: Browse / Clone / Init, search, Open + Recent repo lists |
| `04-file-diff-view.png` | Clicking a changed file: unified diff replaces the graph, File View / Diff View toggle |
| `05-commit-graph-many-branches.png` | Busy repo: many coloured lanes, merge dots, tags, remote-branch pills, author vs committer |
| `06-local-behind-remote.png` | Local `main` behind `origin/main`: separate pills on different rows |
| `07-wip-unstaged-changes.png` | `// WIP` row at top of graph, right panel with Unstaged/Staged files, "Stage All Changes" |
| `08-wip-staged-commit-box.png` | Files moved to Staged, "Unstage All Changes", commit box (Amend, summary + 72-char counter, description, commit button) |

Test repos on this machine:
- `~/wahoo/dex1/mfe/lobby-mfe` — 2,303 commits, 617 refs (100 remote branches, 515 tags), 143 merges. Primary graph stress test (matches screenshot 05).
- Any small repo for basic flows; this repo itself once it has commits.

## Decisions (made with the user)

- **Stack:** Tauri 2 (Rust backend + webview UI). React 19 + TypeScript + Vite frontend.
- **Git reads:** `git2` crate with `default-features = false` (no OpenSSL/SSH — network ops go through the git CLI).
- **Clone / fetch:** shell out to the user's `git` so existing credentials (SSH agent, osxkeychain, GCM) just work.
- **Terminal:** docked panel at the bottom of each repo tab (not the external system terminal), `portable-pty` + xterm.js. Multiple sessions per repo ("Claude", "Shell").
- **Diff view:** included in v1.
- **WIP:** included in v1 **with staging and committing** (user confirmed via screenshot 08). Leave out the AI buttons and a stash UI. **Discard** was added on request (2026-10-11): right-click a file (or a folder in Tree view) in Unstaged Files → **Discard changes…**, always confirmed. Tracked files go back to their staged/committed content (`git restore --worktree`, literal pathspecs), so staged changes are kept; untracked files are deleted, along with folders that become empty; conflicted files are refused. The same right-click menu has Stage/Unstage and Copy file path. (Smart Features creates, applies and drops its own named stashes internally; there is still no general stash browser or stash buttons.)
- **Commit vs. push:** Commit is **local only**. The toolbar **Push** button pushes all local commits on the current branch to its remote (user confirmed). The **Pull** button is in v1 too, **fast-forward only** (user confirmed).
- **Auto-fetch:** every ~60s, fetch remote changes in the background but **never pull** (user confirmed).
- Use `pnpm`. Install versions known to exist (already in `package.json`).

## Status

**v1 is built and verified** (2026-10-10). All features below are implemented.

Verified with automated UI runs (headless Chrome + dev bridge, see CLAUDE.md) against real repos:
- lobby-mfe (read-only): graph with lanes/merges/tags/remote pills, local-behind-remote pills, sidebar
  tree + `5↓`, commit detail with author vs. committer and co-authors, diff view, file view, Path/Tree,
  sidebar tag jump, date separators. Load: ~50 ms for 2,303 commits, 6 lanes, ~880 KB payload.
- Throwaway repos in the scratchpad: WIP row + counts, unstaged/staged lists, WIP diffs, stage per file
  and all (open diff follows the file to the staged side), commit, amend, first commit in an unborn repo,
  Fetch, diverged Pull banner, rejected Push banner, successful Push, fast-forward Pull, push-with-upstream
  popover, background auto-fetch (updates `origin/*` only), warning dot on failed background fetch,
  Launchpad (open/recent/missing), Clone dialog, tab persistence.
- Terminal: Rust test spawns a real pty session (cwd correct, `CLAUDECODE` stripped); login shell resolves
  `claude` from a bare GUI PATH.

Not verified automatically (needs a human in the native window): the xterm.js terminal panel UI with a
live `claude` session, native folder pickers (Browse/Init), macOS title-bar drag region, back-off timing.

**Self-update** (2026-10-14): `tauri-plugin-updater` + `tauri-plugin-process`, same design as Deskwork.
Releases are tag-triggered (`.github/workflows/release.yml`, universal macOS `.dmg` + signed
`.app.tar.gz` + `latest.json` on this repo's GitHub Releases); the endpoint is
`releases/latest/download/latest.json`. `src/updater.ts` checks silently on launch, hourly if the last
check is over 6 h old, and on window focus after 1 h; an update pill appears in the tab bar
(Install → progress → Restart), and Settings has Updates (version + Check for updates). Versions are
kept in step by `scripts/bump-version.mjs` (CI runs `--check` and refuses a tag that disagrees).
Verified: signed local `tauri build` produces the `.app.tar.gz` + `.sig`; Settings section in headless
Chrome (silent launch check under the dev bridge). Not yet verified: an end-to-end update from a
published release.

**Double-click checkout** (2026-10-23): double-click a local branch in the sidebar, a remote branch in
the sidebar, or a graph row to switch to it (`checkout_branch`). A local branch is checked out where it
is, never moved to its upstream (the toast says "N behind origin/x; Pull to update"). With no local
branch, a remote branch becomes a local one tracking it. A graph row picks its first local branch that
isn't checked out, else a remote-only branch (`checkoutTarget` in `src/refs.ts`). `git switch` carries
uncommitted changes and refuses to overwrite them (shown in the error banner); refused during a Smart
Features switch. When the repo's environment has an active feature and the target isn't its branch,
`sf_checkout_warning` drives a confirm dialog: on base, edits move back onto the feature branch and
commits are blocked; on any other branch, the environment can't switch features or go back to base
until the repo returns. Verified: Rust test (stays at the local commit, tracking branch, warning kinds),
`src/refs.test.ts`, and a headless-Chrome run of base / other / feature-branch double-clicks.

Graph lane rule (refined during testing): a commit's first parent folds into an existing lane only when
that lane is to its left; otherwise it continues straight down. Keeps main history in column 0 and the
graph narrow (lobby-mfe: 6 lanes).

## Features (v1)

### Tabs
- Tab bar at the very top, in the macOS title-bar area (Overlay style): pad left ~78px for traffic lights on macOS only; `data-tauri-drag-region` on empty bar space.
- **Launchpad** tab (rocket icon) = Repository Management. Each opened repo = its own closable tab (branch icon + repo name). `+` opens a new Launchpad tab.
- Opening a repo from a Launchpad tab **replaces** that Launchpad tab; if the repo is already open, just activate its tab.
- Closing the last tab leaves a Launchpad tab. Middle-click closes.
- Inactive repo tabs stay **mounted** (hidden with `display:none`) so terminals and scroll survive switching.
- Persist open tabs + active tab; restore on launch (skip paths that no longer open).

### Launchpad (screenshot 03)
- Buttons: **Browse** (native folder picker → open), **Clone**, **Init** (pick folder → `git2::Repository::init` → open).
- Search box filters the lists.
- Sections: **Open repositories**, **Recent repositories** (remove ×). Each row: repo name, owner/org parsed from `origin` URL (e.g. "WahooFitness"), current branch chip, ahead/behind chip. Click/open icon → open in tab.
- **Clone dialog:** URL input (accepts `https://github.com/o/r(.git)`, `git@github.com:o/r.git`, or `o/r` shorthand → expand to `https://github.com/o/r.git`), parent directory (default: last used, else home; Browse button), folder name auto-derived from URL (editable), live progress log from `git clone --progress`, Cancel/Clone. On success open in a tab and remember the parent dir.

### Repo tab layout
```
┌ toolbar: repository <name> › branch <name>   [Fetch] [Refresh]   [Terminal]        ┐
├ sidebar ┬ center: commit list (or diff view)              ┬ right detail panel     ┤
│         │                                                 │                        │
├─────────┴─────────── terminal panel (resizable, hideable) ┴────────────────────────┤
```
- **Toolbar:** repo name, current branch (or "detached HEAD"), ahead/behind. Buttons: **Fetch** (`git fetch --all --prune`), **Pull**, **Push**, **Refresh**, **Terminal** (no sessions → start a Claude session and show panel; else toggle panel). Shortcut: Ctrl+` toggles terminal.
- **Auto-fetch:**
  - Every 60s, for **every open repo tab** (not just the active one), run `fetch_repo` in quiet mode. Stagger tabs by a few seconds so they don't all hit the network at once.
  - Only fetch. Never pull, merge or touch the working tree. Remote-tracking refs, the `↓N` badges and the graph's remote pills update; the local branch stays put.
  - Skip a tick if the repo has no remotes, or a fetch/pull/push/clone is already running for it. Never run two git network ops on one repo at the same time; a per-repo lock lives in the backend.
  - After it finishes, call `repo_fingerprint`; reload refs/graph only if something changed, keeping selection and scroll position.
  - Silent on success: no toast. The Fetch button tooltip says "Last fetched 20s ago".
  - On failure (offline, auth): no banner or toast. Show a small warning dot on the Fetch button with the error in its tooltip. Back off (1 → 2 → 5 → 10 min), and reset to 60s after any success, including a manual Fetch.
  - Settings: `autoFetchSeconds` (default 60, 0 = off), stored in settings.json. A small settings popover (gear icon in the tab bar's right corner) with an on/off toggle and an interval field is enough.
- **Pull button:**
  - Shows a badge with the number of commits behind upstream (e.g. `↓1`, computed from the last fetch). Disabled on a detached HEAD, when the branch has no upstream (tooltip explains), and while running.
  - Runs `git pull --ff-only --progress`, which also fetches, so a pull picks up remote commits even before you press Fetch.
  - Uncommitted changes are fine as long as git can fast-forward around them. If git refuses because local changes would be overwritten, show its message in the error banner.
  - Branch has diverged (local and remote both have new commits) → no merge or rebase. The error banner says the branch has diverged and to merge or rebase in the terminal. Phase 2 may add pull with rebase or merge.
  - On success: show a "Pulled N commits from origin/main" toast (or "Already up to date"), reload graph, refs and WIP, and keep the selection if that commit still exists.
- **Push button:**
  - Shows a badge with the number of commits ahead of upstream (e.g. `↑3`). Disabled on a detached HEAD, and while a push is already running (show a spinner then).
  - Branch has an upstream → `git push`. No upstream yet → a small popover asks for the remote (default `origin`) and branch name (default: the same as local), then runs `git push -u <remote> <local>:<name>`.
  - Never force-push. If the push is rejected (remote has new commits / non-fast-forward), show git's message in a dismissible error banner, suggesting Pull first.
  - On success: show a short "Pushed N commits to origin/main" toast, then reload refs. The local and remote pills merge back into one, and the sidebar `↑` count clears.
  - Nothing to push (ahead = 0 with an upstream) → button still works (git reports "Everything up-to-date"), but render it dimmed.
- **Auto-refresh:** on window focus (and after fetch, and every few seconds while the repo tab is active is optional), call `repo_fingerprint`; reload refs/graph only if it changed. Always re-check WIP status on focus — Claude in the terminal edits files.

### Sidebar (screenshots 02, 06)
- Collapsible (chevron button), filter box (filters branch/tag names; while filtering, expand everything).
- Sections with counts: **LOCAL**, **REMOTE**, **TAGS** (TAGS collapsed by default — lobby-mfe has 515).
- LOCAL: branch icon + name; HEAD branch row highlighted **green with a ✓**; ahead/behind vs upstream shown as `1↑` / `1↓` on the right.
- REMOTE: grouped per remote (GitHub icon if URL is github.com), then a folder tree split on `/` (e.g. `dependabot/github_actions/vendored-acti…`). Skip `origin/HEAD`.
- Clicking any branch/tag selects its target commit and scrolls the graph to it. Double-clicking a branch checks it out (see Status).

### Commit graph list (screenshots 01, 05, 06)
- Columns: **BRANCH / TAG** | **GRAPH** | **COMMIT MESSAGE**. First two widths resizable by dragging header borders. Graph column scrolls horizontally (horizontal wheel / small slider) when lanes exceed width.
- **Virtualized** fixed-height rows (~34px) — render only visible rows + overscan. Must stay smooth on thousands of commits. Cap load at ~20,000 commits with a "truncated" notice.
- Keyboard ↑/↓ moves selection. Default selection = HEAD commit (or WIP if dirty).
- **Graph rendering** (per-row SVG from backend segments):
  - Lane width ~22px; palette: `#15a0bf #0669f7 #8e00c2 #c517b6 #d90171 #cd0101 #f25d2e #f2ca33 #7bd938 #2ece9d` (color index mod length).
  - Vertical segment = straight line. Top→mid segment (lane converging into node): quadratic curve, control `(x1, mid)` — goes down then bends into node. Mid→bottom (fork/merge edge out of node): control `(x2, mid)` — goes sideways then bends down.
  - Normal commit node: author avatar circle (~22px) with a ring in the lane color. **Merge commit** (2+ parents): small solid dot, no avatar.
  - Faint tint of the lane color on the row from node to message; message cell has a 3px left border in the lane color. Selected row: blue highlight.
- **Ref pills** (BRANCH/TAG column): background = lane color (translucent) with border; thin connector line from pill across to the node.
  - Local branch: laptop icon; checked-out branch also gets a ✓. Remote branch: GitHub icon (cloud icon for non-GitHub). Tag: tag icon.
  - Local + its same-named remote at the **same commit** → one pill with both icons. If they differ (local behind/ahead), separate pills on their own rows (screenshot 06).
  - Long names truncated with `…`. Multiple refs: show the most important (HEAD > local > remote > tag) plus `+N` badge; tooltip lists all.
- **Message column:** summary (bright) + body flattened to one line with ` | ` separators (dim), ellipsized. Hover tooltip shows full summary + body.
- **Date separators:** when the relative bucket changes (today, yesterday, N days ago, N weeks ago, a month ago, N months ago, N years ago) show a small right-aligned label like `4 days ago` and a faint separator line.
- **Avatars:** GitHub noreply emails (`123+name@users.noreply.github.com`) → `https://avatars.githubusercontent.com/u/123?s=64`; otherwise Gravatar `https://www.gravatar.com/avatar/<md5>?s=64&d=404`; on error fall back to coloured initials (color hashed from email). Remember failed URLs in a module-level Set to avoid refetch storms while scrolling.

### WIP row (screenshot 07)
- If the working tree has changes (staged, unstaged, or untracked; ignore ignored files), show a pinned first row `// WIP` with a **dashed hollow circle** node in the HEAD branch's lane, connected by a dashed line down to the HEAD commit, and counts `✎ 4 ✚ 2` (modified / added; `✖ n` deleted if any).
- Selecting WIP → right panel shows "**N file changes on `main`**", Path/Tree toggle, sections **Unstaged Files (n)** and **Staged Files (n)**. Untracked = added (green +). Clicking a file opens its diff in the center: unstaged = index→workdir (untracked: whole file as additions), staged = HEAD→index.
- **Staging (screenshots 07, 08):**
  - Section headers have **Stage All Changes** (green outline, on Unstaged) / **Unstage All Changes** (red outline, on Staged); hidden when the section is empty.
  - Hovering a file row shows a per-file **Stage** / **Unstage** button. Both sections are collapsible and scroll independently; the commit box stays pinned at the bottom.
  - A file can be in both lists (partially staged). After every stage/unstage, re-read `wip_status`. If the file whose diff is open moves to the other list, switch the diff to that side.
- **Commit box** (pinned at the bottom of the WIP panel):
  - **Amend previous commit** checkbox. When ticked, prefill summary/description from HEAD's message, and allow committing with nothing staged.
  - **Commit summary** input with a remaining-characters counter starting at 72 (goes negative and turns amber/red past 72), plus a multi-line **Description** textarea.
  - The big button's state: nothing staged → disabled "Stage Changes to Commit". Staged but empty summary → disabled "Type a Message to Commit". Otherwise enabled "Commit Changes to N Files" (or "Amend Previous Commit"). Cmd/Ctrl+Enter commits.
  - On success: clear the inputs, reload the repo (graph, refs, WIP) and select the new HEAD commit. On failure (e.g. a hook rejects the commit), show git's stderr inline and keep the message.
  - Keep the draft message per repo in memory, so switching tabs doesn't lose it.

### Commit detail panel (screenshots 01, 05)
- Header `commit: 3a9899` (click to copy full SHA).
- Message box: summary (large) + body (pre-wrap), scrollable.
- Author avatar + name, `authored 10/09/2026 @ 11:05 AM`; `parent: 526349` (each parent clickable → select). If committer differs from author (e.g. GitHub), show second row `committed …`.
- **Co-authors:** parse `Co-authored-by: Name <email>` trailers (case-insensitive), show small avatars with tooltips.
- Stats line: `✎ 1 modified ✚ 6 added ✖ n deleted ↪ n renamed`.
- File list with **Path** / **Tree** toggle. Path mode: status icon (✚ green added, ✎ amber modified, ✖ red deleted, ↪ renamed), directory dim + middle-ellipsized, basename bright (never truncated). Tree mode: collapsible folders. Optional +/− line counts.
- Panel width resizable by dragging its left edge.

### Diff view (screenshot 04)
- Clicking a file (commit or WIP) replaces the graph area (keep the list mounted, hidden). Header: status icon + path, **File View / Diff View** toggle, close × (Esc also closes).
- Diff View: unified, hunk headers `@@ -80,4 +80,5 @@`, two line-number gutters, green/red line backgrounds, monospace. "No newline at end of file" markers. Binary → "Binary file" notice. Cap ~20k lines with a notice.
- File View: full file content at that commit (or workdir for WIP) with line numbers; deleted files show the parent's version.

### Terminal panel
- Bottom panel spanning the repo tab, resizable height (drag top edge), hide button. Session tabs ("Claude 1", "Shell 1"), `+ Claude`, `+ Shell`, close a session (kills the pty).
- Claude session (unix): spawn `$SHELL -l -i -c "claude; exec '<shell>' -l -i"` in the repo root so PATH from the user's dotfiles applies and the shell remains after Claude exits. Shell session: `$SHELL -l -i`. Windows: `powershell.exe -NoLogo -NoExit -Command claude` / `powershell.exe -NoLogo`.
- Env: `TERM=xterm-256color`, `COLORTERM=truecolor`, **remove `CLAUDECODE`** (and other `CLAUDE_CODE_*` session vars) so a nested `claude` doesn't refuse to start when the app is launched from a Claude session.
- xterm.js with FitAddon, dark theme matching the app, font Menlo / SF Mono / Consolas 13px. Refit on container resize (ResizeObserver) and whenever the panel/tab becomes visible (hidden xterm can't measure). Send resize to the pty when cols/rows change.
- Unmounting a repo tab kills its sessions.

## Backend (Rust) — `src-tauri/src/`

- `lib.rs`: register `tauri_plugin_dialog`, `.manage(pty::PtyState::default())`, all commands below.
- All git commands are `async` and run their body in `tauri::async_runtime::spawn_blocking` (open `Repository` inside the closure; it is not shared). Errors → `Result<T, String>`.
- Serialize with `#[serde(rename_all = "camelCase")]`.

### `git.rs` commands
| Command | Returns |
|---|---|
| `open_repo(path)` | `{root, name}` via `Repository::discover`; root = workdir without trailing slash |
| `repo_summary(path)` | `{root, name, branch, ahead, behind, owner}` — owner parsed from origin URL (for Launchpad) |
| `load_repo(path)` | `RepoData` (below) |
| `repo_fingerprint(path)` | string hash of HEAD + every ref name/target (cheap change detection) |
| `wip_status(path)` | `{unstaged: FileChange[], staged: FileChange[]}` via `repo.statuses` (include untracked, recurse untracked dirs, exclude ignored); empty = clean |
| `commit_detail(path, oid)` | `CommitDetail` |
| `file_diff(path, oid, file, oldFile?)` | `FileDiff` for a commit file |
| `wip_file_diff(path, file, staged)` | `FileDiff` for a WIP file |
| `file_content(path, oid?, file)` | `{binary, content, truncated}`; `oid` null = workdir; falls back to first parent if missing at oid |
| `clone_repo(url, parentDir, name)` | runs `git clone --progress`, emits `clone-progress` events (split stderr on `\r`/`\n`), env `GIT_TERMINAL_PROMPT=0`; returns `{root, name}`; error includes last stderr lines. Refuse non-empty existing dest |
| `init_repo(path)` | `{root, name}` |
| `fetch_repo(path, quiet)` | runs `git fetch --all --prune` (`--quiet` when background). Env `GIT_TERMINAL_PROMPT=0`, `GCM_INTERACTIVE=never`, and a hard **timeout (~45s, kill the child)** so a background fetch can never hang waiting on a credential/SSH prompt. Shares the per-repo network lock with pull/push; returns `{skipped: true}` if the lock is held |
| `stage_files(path, files)` / `unstage_files(path, files)` | `git add -A -- <files>` / `git restore --staged -- <files>` (no HEAD yet → `git rm --cached -r -- <files>`); an empty `files` = all |
| `pull_repo(path)` | runs `git pull --ff-only --progress`, env `GIT_TERMINAL_PROMPT=0`, emits `pull-progress` events; returns `{before, after}` HEAD shas so the UI can count pulled commits; error = stderr (diverged / would overwrite local changes) |
| `checkout_branch(path, branch, track?)` | `git switch <branch>` if it exists locally (never moves it), else `git switch -c <branch> --track <track>`; refused during a Smart Features switch |
| `push_repo(path, remote?, branch?)` | runs `git push --progress` (with `-u <remote> HEAD:<branch>` when setting an upstream), env `GIT_TERMINAL_PROMPT=0`, emits `push-progress` events; error = stderr. Never `--force` |
| `head_message(path)` | HEAD's full message (for Amend prefill) |
| `commit(path, message, amend)` | `git commit -F -` with the message on stdin (`--amend` if set) so hooks, signing and the user's identity config all apply; returns new HEAD sha; error = stderr |
| `default_clone_dir()` | home directory |

On Windows, spawn git with `CREATE_NO_WINDOW` (`0x08000000`) via `CommandExt::creation_flags`.

`RepoData`:
```
root, name, fingerprint, truncated,
head: {branch: string|null, oid: string|null, detached: bool},
authors: [{name, email, gravatar /* md5 of trimmed lowercase email */}],
commits: [{id, summary, body /* truncated ~400 chars */, author /* index into authors */,
           time /* author time, unix secs */, merge, lane, color,
           segs: [[x1, y1, x2, y2, color]] /* y: 0 top, 1 mid, 2 bottom */,
           refs: [{name, kind: "local"|"remote"|"tag", remote: string|null, isHead}]}],
maxLanes,
locals:  [{name, target, upstream, ahead, behind, isHead}],
remotes: [{remote, name /* without remote prefix */, fullName, target}],
tags:    [{name, target}]
```
- Revwalk: `Sort::TOPOLOGICAL | Sort::TIME`; push every local/remote/tag target (peel annotated tags to commits; skip tags on non-commits) plus HEAD. Skip symbolic refs like `origin/HEAD`. Remote name = longest configured remote that prefixes the branch name.
- Use lossy UTF-8 for messages/names (`message_bytes`, `name_bytes`).
- Intern authors by (name, email) to keep the payload small.

`CommitDetail`: `{id, summary, body, author: Person, committer: Person, parents: [sha], coAuthors: [{name, email, gravatar}], files: FileChange[], filesTruncated}` with `Person = {name, email, gravatar, time}` and `FileChange = {path, oldPath, status: "A"|"M"|"D"|"R"|"C"|"T", additions, deletions, binary}`. Diff against first parent (root commit → empty tree), `find_similar` for renames, line stats via `git2::Patch::from_diff(..).line_stats()`; cap ~5,000 files.

`FileDiff`: `{binary, truncated, hunks: [{header, lines: [{o /* '+', '-', ' ', or '\\' for no-newline */, a /* old line no|null */, b /* new line no|null */, t /* text without trailing newline */}]}]}`. Restrict the diff with a pathspec of `[file, oldFile]`, run `find_similar`, pick the delta whose new path == file.

### Graph layout (`graph.rs`)
`GraphBuilder::push(oid, parents) -> RowLayout {lane, color, segments}`. Lanes are `Vec<Option<(expected_oid, color)>>`. For each commit: lanes expecting it converge into the leftmost (node lane); none → new lane + new color (branch tip). Pass-through lanes emit full-height verticals. First parent folds into a lane to its left that already expects it (freeing the node lane), otherwise continues the node lane; extra parents join existing lanes or allocate new lanes with new colors. Freed slots are reused (no compaction); trailing empty lanes trimmed; `max_lanes` tracked.

### `pty.rs`
- `PtyState { sessions: Mutex<HashMap<u32, Session>>, next: AtomicU32 }`, `Session { master, writer, child }`.
- `pty_spawn(cwd, kind: "claude"|"shell", cols, rows) -> id`; reader thread reads 16KB chunks and emits `pty-data` `{id, data: base64}`; on EOF emits `pty-exit` `{id}`.
- `pty_write(id, data)`, `pty_resize(id, cols, rows)`, `pty_kill(id)`.
- **Frontend must register one global `pty-data` listener at startup and buffer data for ids that don't have a handler yet** — output can arrive before `pty_spawn` resolves.

### `settings.rs`
`load_settings() -> JSON|null`, `save_settings(json)` → `<app_config_dir>/settings.json` (write tmp file then rename). Frontend owns the schema:
`{recent: [{path, name, lastOpened}], openTabs: [path], activeTab: path|null, cloneDir?: string, autoFetchSeconds?: number}`.

## Frontend — `src/`

- `main.tsx`, `App.tsx` (tabs state, settings, open/close/activate), `api.ts` (typed `invoke` wrappers + pty event bus), `types.ts`, `styles.css` (dark theme tokens: app bg ~`#1e2128`, panels ~`#24272e`, title/tab bar ~`#2b2f36`, text ~`#d4d7dd`, dim ~`#8b919c`, accent ~`#1f6feb`).
- `components/`: `TabBar`, `Launchpad`, `CloneDialog`, `RepoView` (toolbar, layout, data loading, focus refresh), `Sidebar`, `CommitList` (virtualized list + `GraphCell` + `RefPills`), `DetailPanel` (commit + WIP modes), `FileList` (Path/Tree), `DiffView`, `TerminalPanel` + `TerminalView`, `Avatar`, `icons.tsx` (inline SVG icons: branch, folder, cloud, laptop, tag, github, terminal, refresh, fetch, plus, x, chevron, search, file, list, tree, rocket, check, pencil).
- `index.html`, `vite.config.ts` (port 1420, `strictPort`, ignore `src-tauri` in watch), `tsconfig.json` (strict, `jsx: react-jsx`, `moduleResolution: bundler`, `types: ["vite/client"]`). Import `@xterm/xterm/css/xterm.css`.

## Icons
Make `app-icon.svg` (simple git-branch glyph on a rounded square) and run `pnpm tauri icon app-icon.svg`. If SVG isn't accepted, rasterize to a 1024px PNG first (e.g. `qlmanage -t -s 1024 -o . app-icon.svg` on macOS).

## Verification
1. `cd src-tauri && cargo test` (graph tests) and `cargo build`.
2. `pnpm typecheck`.
3. `pnpm tauri dev`, then check: open lobby-mfe → graph matches screenshot 05 in spirit (lanes, merge dots, tags, pills), scroll is smooth, clicking commits shows detail/co-authors/committer, file click shows diff, sidebar click jumps to branch; in a **throwaway repo** (`git init` in the scratchpad, never lobby-mfe or the user's repos) → WIP row, unstaged/staged lists, WIP diffs, stage/unstage per file and all, commit, amend, hook-failure message; push to a throwaway bare remote (`git init --bare` in the scratchpad, added as `origin`): first push sets upstream via the popover, later pushes use plain `git push`, a rejected push shows the error banner; pull: make a second clone of the bare remote, push a commit from it, then Pull in the app fast-forwards (badge `↓1` first after Fetch); create divergence and confirm Pull shows the diverged banner without modifying anything; auto-fetch: push a commit from the second clone, wait ~60s without touching the app, and confirm `origin/main` moves and `↓1` appears while local `main` and the working tree are unchanged; point `origin` at a bad URL and confirm only the warning dot appears, with back-off; clone a small public GitHub repo → opens in a new tab; Terminal → `claude` starts in repo root; tabs persist across restart.
4. Screenshot the running app to compare against `docs/reference/` (use the `run` skill).
5. Commit in logical chunks once it works (ask the user before the first commit).

## Smart Features (approved; Phases 1–3 built)

Approved by the user on 2026-10-11 as an exception to "keep it simple". The full plan lives in the
user's workspace task `~/personal/brett-work/completed/2026-10-10-dex-worktrees.md`. Summary:

- **Smart Environment**: a user-defined set of local repos from anywhere on disk (e.g. the dex repos,
  or the RGT server repos + game client). A repo belongs to only one environment. Not tied to dex or dexman.
- **Smart Feature**: a branch name the user types (validated by a regex implementing git's ref-name
  rules, then `git check-ref-format --branch`). A repo's feature branch is created **only when a file
  in that repo changes** (filesystem watcher → `git switch -c`), never up front.
- **Switching features** happens in place: no git worktrees, no extra clones or folders. It stashes the
  current feature's uncommitted work per repo (pinned under `refs/smart-features/<id>/stash`), returns
  repos to their base branch, then checks out the other feature's branch and re-applies its stashes. It
  runs as a journaled, resumable/rollback-able transaction.
- **Storage**: git holds the real state (branches, pinned stashes). `<app_config_dir>/smart-features/`
  holds `environments.json` + one `env-<id>.json` per environment (Rust-owned schema, atomic writes).
  `settings.json` keeps only panel UI state.
- **Phases**: 1 environments + read-only status panel; 2 features + auto-branch; 3 switching + finish;
  4 polish (see **Phase 4 checklist** below).

**Status (2026-10-11, branch `feat/smart-features`):** Phase 1 is built and verified.
- Backend `src-tauri/src/smart.rs`: `SmartStore` (one `env-<id>.json` per environment plus an
  `environments.json` order file, atomic writes with a `.bak`, unreadable file → falls back to the
  backup with a warning), commands `sf_list_envs`, `sf_save_env`, `sf_delete_env`, `sf_validate_repo`,
  `sf_scan_folder`, `sf_status`. Repos are stored by real path, so a symlinked path is the same repo.
  One environment per repo is enforced on save. Edits never overwrite feature state (features, active
  feature, journal), and removing a repo that a feature uses is refused.
- Base branch: per-repo override → shared base (if the repo has it) → `origin/HEAD` →
  develop/main/master.
- Dots (computed in Rust): red = staged/unstaged/untracked/conflicted; yellow = ahead of upstream,
  or ahead of base with no upstream; green otherwise. Amber warning for merge/rebase in progress,
  detached HEAD, no base branch, or not on the expected branch.
- UI: `SmartFeaturesPanel` (left, resizable, toggled by the layers icon in the tab bar; clean repos
  collapse into "N clean"; Changed only filter; Fetch all; Pull all (fast-forward only, skips repos with no upstream); Merge base into feature (active feature only: fetches each repo's base and merges `origin/<base>` into the feature branch with `git merge --no-edit`; skips repos with uncommitted changes and aborts a merge that conflicts, so no repo is left half-merged); Edit/Delete) and `EnvironmentDialog`
  (Add repo…, Add from folder… with a checklist, Add open tabs, labels, base overrides, shared base).
  Panel UI state lives in `settings.json` as `smartFeatures`.
- Deviations from the plan: which environment holds a repo is derived from the env files rather than
  stored in a `repoIndex`, so the two can't drift; the cross-process lock is an `flock` on
  `smart-features/.lock` (added 2026-10-11 with the skills), taken for every write; environment repos without an open tab are refreshed by the panel (focus +
  every 30 s) and fetched by **Fetch all**, not by background auto-fetch yet.
- Verified: 6 Rust tests (`cargo test smart`) and a headless-Chrome run through the dev bridge
  against throwaway repos (create via Add from folder + Add open tabs, dots for each state,
  one-environment-per-repo note, open repo tab from the panel, delete). The dev bridge stores
  environments under `GDT_BRIDGE_CONFIG_DIR` and answers folder pickers with `GDT_BRIDGE_PICK`.

**Status (2026-10-11): Phase 2 is built and verified.**
- `src-tauri/src/smart_features.rs`: `sf_check_branch` (git's `check-ref-format`, not a base or an
  existing feature's branch, `a` vs `a/b` ref conflicts, where it exists locally/on origin, which
  repos are dirty on base), `sf_start_feature` (creates no branches; `adopt` takes over an existing
  branch and checks it out where it exists; repos already dirty on base move right away),
  `sf_back_to_base`, `sf_move_to_feature`, `sf_commit_guard`, `sf_checkout_remote`,
  `sf_rename_feature`, `sf_push_all` (`git push -u origin <b>:<b>` where there are unpushed
  commits, 4 at a time, never forced; works for inactive features).
- Auto-branch: when a repo is dirty **on its base** and its environment has an active feature,
  `git switch --no-track -c <branch>` (or `git switch <branch>` if it already exists locally). It never
  bases the branch on `origin/<branch>`; that repo shows a warning instead. A repo found on the
  feature branch by hand is adopted into the feature. Feature operations share one lock
  (`SmartStore::op_lock`).
- `src-tauri/src/smart_watch.rs`: one `notify` watcher (FSEvents on macOS) over every environment
  repo, 300 ms debounce (flushes at least every 2 s), ignores events where every path is gitignored,
  plus a sweep every 10 s. Emits `sf-repo-changed`, `sf-auto-branched`, `sf-auto-branch-failed` (once
  per distinct error). Resynced when environments are saved or deleted, and on every sweep.
- Commit guard: `git::commit` refuses to commit on base while the repo's environment has an active
  feature; the commit box shows "Move to <branch> and commit".
- UI: active-feature bar per environment (name, branch, repo count, unpushed, Push all), "has
  origin/<branch>: Check out" rows for a teammate's branch, a Features list for inactive features, menu
  items New feature… / Push all / Rename feature… / Back to base, and `NewFeatureDialog` (live
  validation via `src/branchName.ts`, adopt, "Pull base branches first"). Repo tabs refresh right away
  when Smart Features changes a branch (`gdt-repos-changed` window event).
- Phase 2 limits (Phase 3 removes them): **Back to base** and starting another feature refuse while the
  active feature's repos have uncommitted changes (no parking yet); a feature that never touched a
  repo is dropped when it's deactivated; there's no Switch or Finish yet, so a repo that a feature
  used can't be removed from its environment.
- Verified: `cargo test` (22 pass; Smart Features tests cover start, auto-branch keeping staged/
  unstaged/untracked changes, name checks, adopt, commit guard, back to base, Push all, remote
  checkout, and a real watcher test that edits a file and sees the branch appear); `pnpm test` runs
  `src/branchName.test.ts`, which compares the regex with the installed git; and headless-Chrome runs
  through the dev bridge (validation messages, start, auto-branch on an external edit, Push all, Back
  to base, teammate branch Check out, commit guard). The bridge runs the watcher too
  (`GDT_BRIDGE_NO_WATCH=1` turns it off) and refuses feature commands unless every repo of the
  environment is under `GDT_BRIDGE_WRITE_ROOT`.

**Status (2026-10-11): Phase 3 is built and verified.**
- `src-tauri/src/smart_switch.rs`: a switch to feature T (or to base) *converges* every repo from its
  current state: **park** (a repo on feature F's branch runs `git stash push --include-untracked`, tagged
  `[sf <F-id>]`, pinned with `git update-ref refs/smart-features/<F-id>/stash`), **base** (`git switch
  <base>`), **switch** (`git switch <T-branch>` where T's branch exists), **restore** (`git stash apply
  --index <pin>`, falling back to a plain apply when the index can't be restored; on success the pin and
  its stash-list entry are dropped). Each step checks git first, so running it again is safe: Retry =
  switch to the journal's `to` again, Roll back = switch to its `from`. A stash made just before a crash
  is found by its tag and pinned.
- Preflight (read-only after an auto-branch pass) refuses, listing problems, for: missing folder,
  merge/rebase in progress, `index.lock`, unresolved conflicts, detached HEAD, no base, a repo on a
  branch that's neither base nor a feature's, uncommitted changes on base in a repo the target uses, or
  parked changes left from a conflicted restore. The journal (`env.journal`: from, to, steps with
  status) is written before the first step and after each; it's cleared on success. While it exists,
  commits, auto-branch and other switches in that environment are refused, and the panel shows a banner
  with **Retry** / **Roll back**. Fetch/pull/push locks are held for every repo during a switch.
- A conflicting restore keeps the pin, leaves the conflict markers, and finishes the rest of the switch;
  the repo shows "parked changes conflicted" with **Mark resolved** (allowed once no conflicts remain).
- **Back to base** and starting a feature while another is active now park instead of refusing. Features
  with no repos are kept (Finish removes them).
- **Finish** (`sf_finish_preview`, `sf_finish_feature`): per repo, local branch, pushed/unpushed,
  merged into `origin/<base>`, parked changes. Unpushed-and-unmerged commits or parked changes block
  unless that repo is ticked to discard. If the feature is active it goes back to base first; then pins
  and stash entries are dropped and local branches deleted. Remote branches are never deleted.
- UI: Features list rows have **Switch** and ⋯ (Push all, Rename…, Finish…) and show 📦 for parked
  changes; `SwitchFeatureDialog` (preflight summary per repo, problems, live step ticks from
  `sf-progress`); `FinishFeatureDialog`; the recovery banner.
- Verified: `cargo test` (28 pass; switching tests cover A→B→A restoring staged, staged+unstaged and
  untracked files exactly, pins surviving `stash clear` + `reflog expire` + `gc --prune=now`, an
  interrupted switch rolled back and retried, the stash-before-crash case, a conflicting restore +
  Mark resolved, preflight blocking, and Finish blocking then discarding without touching origin). A
  headless-Chrome run through the dev bridge did: start a second feature (parks the first), switch
  back with everything restored, a blocked preflight (`index.lock`), a switch failed mid-way by a
  `reference-transaction` hook → banner → Retry, and Finish requiring ticks.
- Not done: see the **Phase 4 checklist** below.

**Claude Code skill: `smart-environment` (2026-10-11).** The app ships a Claude Code plugin in
`src-tauri/claude-plugin/` (bundled as resources; in dev it's read from the source folder). Terminals
opened in the app load it: the Claude terminal starts `claude` with `--plugin-dir`, and in zsh a
`$ZDOTDIR` wrapper (`pty.rs` `ZSH_FILES`) sources the user's own zsh files and then defines
`claude()` to add `--plugin-dir`; it restores the user's `ZDOTDIR` at the end so nested shells are
untouched. The skill asks for the name, which repos and an optional shared base, then runs
`smart_env.py` (Python 3 stdlib), which scans a folder (any depth), applies the app's rules (unique
name, one environment per repo, real paths, non-bare) and writes `env-<id>.json` + the index. It only
creates environments; edits stay in the app. The watcher emits `sf-envs-changed` when the set of
environments changes on disk, so the panel shows it within ~10 s.

A second skill, `smart-environment-add-repo`, handles "add crux_common to RGT": `smart_env.py
find-repo` (repos in environments by folder name or label, then a scan of given folders) and
`add-repo --env E --repo P --move`. A repo can be in only one environment (one working folder, one
checked-out branch), so adding moves it. The move is refused while the old environment has an
unfinished switch or any feature there touched the repo (recorded in the feature, a pinned stash, or
the feature's local branch exists); the fix is Finish in the app. It removes the repo from the old
environment before adding it to the new one, so a crash leaves it in neither, never both. The repo
keeps its id, label and base override. Both skills share `claude-plugin/scripts/smart_env.py` and
take the same `flock` as the app.

### Phase 4 checklist (polish; not started)

Tracked as a reminder in the user's workspace task
`~/personal/brett-work/inbox/2026-10-11-smart-features-phase-4.md`. Build when asked. Suggested order: use Smart Features on RGT/dex1 first, then 2, 3 and 6.

From the original plan:
- [ ] 1. **Export / import environments**: save an environment's definition (repos + settings, no
  feature state) to a file; import it on another machine or for a teammate, remapping paths.
- [ ] 2. **Update from base**: show `base ↓N` on a feature when `origin/<base>` has moved on; per
  repo, merge `origin/<base>` into the feature branch (confirmed; stops on conflicts).
- [ ] 3. **Move commits to feature**: when a commit lands on base while a feature is active (e.g.
  `git commit` in a terminal), the repo goes ⚠ with this action: create/advance the feature branch to
  that commit, then `git branch -f <base> origin/<base>` while on the feature branch, so the working
  tree is untouched. Always confirmed.
- [ ] 4. **Launchpad section**: Smart Environments on the start screen for quick access.

Found while building Phases 1–3:
- [ ] 5. **Orphan pins**: find `refs/smart-features/<id>/stash` whose feature no longer exists (e.g.
  the env file was lost) and offer **Restore as feature** or **Drop**.
- [ ] 6. **Locate…**: for a repo whose folder is missing, re-point its path (features reference repos
  by id, so only `path` changes) instead of remove + re-add.
- [ ] 7. **Terminal warning**: the switch dialog warns when git-desktools has terminals open in repos
  the switch will change.
- [ ] 8. **Background auto-fetch for environment repos** without an open tab (today only **Fetch
  all**), with the existing back-off and network locks.
- [ ] 9. **Pull bases (ff-only)** as an environment ⋯ menu action (today only a checkbox when starting
  a feature).

Edge cases:
- [ ] 10. **Submodules**: `git switch --recurse-submodules` for repos that have them.
- [ ] 11. **Git LFS**: warn when a repo uses LFS but `git lfs` isn't installed; show smudge time in the
  switch progress.

- Next: a manual run with the real RGT repos, then Phase 4 items when asked.

## Phase 2 (not in v1 unless asked)
Partial (hunk/line) staging, pull with rebase/merge, stash UI (Smart Features stashes are separate, see above), commit search/filter, blame/history, split diff, syntax highlighting, watching `.git` with a filesystem watcher instead of focus polling, OS notifications when auto-fetch finds new commits.
