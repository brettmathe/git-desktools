---
name: smart-environment-add-repo
description: Add a git repo to an existing git-desktools Smart Environment, moving it out of whichever environment holds it now. Use when the user says things like "add crux_common to the RGT smart environment", "move lobby-mfe into dex1", or "put this repo in RGT".
---

# Add or move a repo into a Smart Environment

A repo can belong to only one Smart Environment, because it has one working folder and one checked-out
branch. "Add X to E" therefore means: find X, take it out of its current environment if that's safe,
and add it to E. The plugin's helper does the checks and writes, under the same lock the app uses:
`python3 <this skill's directory>/../../scripts/smart_env.py`. Every command prints JSON.

## Steps

1. **Find the target environment.** Run `smart_env.py list`. Match the name case-insensitively. If
   none matches, show the existing names and ask which one was meant (or offer the
   `smart-environment` skill to create it). Don't create one silently.

2. **Find the repo.** If the user gave a path, use it. Otherwise run
   `smart_env.py find-repo <name> --search <folder> ...`. Search the folders of the target
   environment's repos and their parents, `~/wahoo`, `~/personal` and the current directory, at
   the default depth (4). It also matches repos already in any environment, by folder name or label.
   - **No match:** say where you looked and ask for the path. Don't guess.
   - **Several matches:** ask which one with AskUserQuestion, showing each path and its environment.

3. **Dry run:** `smart_env.py add-repo --env "<env>" --repo <root> --move --dry-run`.
   - **Already in the target:** say so and stop.
   - **Blocked by feature work** in its current environment (a feature branch, parked changes,
     the active feature, or an unfinished switch): stop and relay the reasons. The fix is in the
     app: finish that feature (Finish… in the panel), or switch away and finish it. Don't delete
     branches or stashes yourself.
   - **No other error:** continue.

4. **Confirm only if it's a move.** If the dry run shows `from` (it's leaving another environment),
   say "Move crux_common from dex1 to RGT?" and wait for a yes. A repo in no environment is just
   added, without asking.

5. **Apply:** the same command without `--dry-run`. Report "<repo> is now in <env>" (and "removed
   from <old env>" for a move) plus any `notes`. A note is, for example, that the repo isn't on its
   base branch, or that its uncommitted changes will move onto the target's active feature. The
   git-desktools panel shows the change within about 10 seconds.

## Rules

- Never edit the environment files by hand, and never run git commands that change the repo.
- The repo keeps its display name and base-branch override when it moves; both can be changed in
  the app's Edit environment dialog.
- `--config-dir` (or `GDT_SMART_CONFIG`) points the helper elsewhere. Use it only for testing,
  with a scratch folder.
