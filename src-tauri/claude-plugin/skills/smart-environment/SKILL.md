---
name: smart-environment
description: Create a git-desktools Smart Environment (a named set of local git repos for Smart Features) from repos found in a folder. Use when the user asks to create, set up or build a Smart Environment, e.g. "create a smart environment for all repos in ~/wahoo/dex1", or asks which environments exist.
---

# Create a Smart Environment

A Smart Environment is a named set of local git repositories in git-desktools. The plugin's helper
`smart_env.py` finds the repos, applies the app's rules (a unique name, one environment per repo,
real paths, non-bare repos) and writes the environment file the app reads, under the same lock the
app uses. It never changes the repositories.

Run it with `python3 <this skill's directory>/../../scripts/smart_env.py`. Every command prints JSON.

## Steps

1. **Work out the folder(s) and depth** from the request. `~` is fine.
   - "recursively", "all subdirectories", "everything under": `--max-depth 0` (unlimited).
   - Otherwise use the app's default, `--max-depth 3`.
   - The scan never descends into a repo (nested repos and submodules belong to it) and skips
     hidden folders, `node_modules`, `build`, `dist`, `target`, `Pods` and `DerivedData`.

2. **Scan:** `smart_env.py scan <folder> --max-depth N` for each folder. Each hit has `root`,
   `name`, `relative`, `branch`, `baseBranch` and `inEnv`, the environment that already holds it.
   If nothing is found, say so and stop.

3. **Show what was found** before asking anything. Group repos by their first path segment under the
   folder, with the count, for example: "android (14): android_gradle, api, bolt, ...". Call out:
   - repos with `inEnv` set: they're skipped, because a repo can only be in one environment;
   - repos with no `baseBranch`: the panel flags them until a base is set in Edit environment.

4. **Ask the required questions** with AskUserQuestion, in one call:
   - **Environment name** (required). Offer the folder's name as the first option, plus one or two
     sensible alternatives. Run `smart_env.py list` first: names must be unique (case-insensitive).
   - **Which repos**: all found (recommended), or let the user list exclusions. If they choose
     exclusions, ask them to name the repos or groups to leave out.
   - **Shared base branch** (optional): "None (use each repo's default)" first, then e.g. a release
     branch if the user mentioned one. It applies only to repos that have that branch.
   Don't ask about anything else. Labels default to folder names, and base branches resolve
   automatically. Mention that both can be changed later in the app's Edit environment dialog.

5. **Dry run, then create.** Write the chosen repo roots as a JSON list to a scratch file and run
   `smart_env.py create --name "<name>" --repos <file> [--shared-base <b>] --dry-run`.
   If it reports an error (duplicate name, a repo already in another environment), fix the input
   or ask the user, and dry-run again. Then run the same command without `--dry-run`.

6. **Report** the environment name and repo count. The git-desktools Smart Features panel shows
   it within about 10 seconds (or click its refresh button). Open the panel with the layers icon
   at the left of the tab bar if it's closed.

## Rules

- Only create new environments. To add or move a single repo, use the
  `smart-environment-add-repo` skill. To rename an environment, change base branches, remove repos
  or delete it, send the user to the panel's ⋯ menu (Edit environment / Delete environment). The app guards
  feature state that this helper doesn't know about.
- Never edit the files in the config folder by hand, and never run git commands that change a repo.
- `--config-dir` (or `GDT_SMART_CONFIG`) points the helper elsewhere. Use it only for testing,
  with a scratch folder.
