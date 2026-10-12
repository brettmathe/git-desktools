#!/usr/bin/env python3
"""Create git-desktools Smart Environments from the command line.

Writes the same files the app does (git-desktools/src-tauri/src/smart.rs):
  <config>/environments.json   {"version": 1, "order": [env ids]}
  <config>/env-<id>.json       one environment
and enforces the same rules: a name (unique, case-insensitive), real paths,
non-bare repos, no duplicates, and a repo belongs to only one environment.
The running app picks a new environment up on Refresh (it adopts env files).
It never touches the repositories themselves.

  smart_env.py list
  smart_env.py scan <folder> [--max-depth N]        N=0: unlimited (default 3)
  smart_env.py create --name NAME --repos FILE|- [--shared-base B] [--dry-run]
      --repos: JSON list of paths, or of {"path", "label"?, "baseBranch"?}
  smart_env.py find-repo NAME [--search DIR ...] [--max-depth N]
  smart_env.py add-repo --env NAME --repo PATH [--move] [--dry-run]
      --move: take it out of the environment that holds it (refused while a
      feature there has a branch, parked changes or a switch in progress)
  Common: --config-dir DIR (default: the app's config folder)

Writes hold an exclusive flock on <config>/.lock, the same lock the app takes,
so the app and this helper never overwrite each other's changes.
"""

import argparse
import contextlib
import fcntl
import json
import os
import subprocess
import sys
import time

DEFAULT_CONFIG = os.path.expanduser(
    "~/Library/Application Support/com.brettmathe.gitdesktools/smart-features"
)
VERSION = 1
SKIP = {"node_modules", "target", "build", "dist", "Pods", "DerivedData"}
BASE_CANDIDATES = ["develop", "main", "master"]
_counter = [0]


def fail(msg):
    print(json.dumps({"error": msg}))
    sys.exit(1)


def new_id(prefix):
    _counter[0] += 1
    return "%s_%x%x" % (prefix, time.time_ns() & 0xFFFFFFFFFFFF, _counter[0])


def canonical(path):
    return os.path.realpath(os.path.expanduser(path)).rstrip("/")


def git(cwd, *args):
    r = subprocess.run(["git", *args], cwd=cwd, capture_output=True, text=True)
    return r.stdout.strip() if r.returncode == 0 else None


def repo_root(path):
    """Top of the working tree containing path, or None (also for bare repos)."""
    if not os.path.isdir(path):
        return None
    if git(path, "rev-parse", "--is-bare-repository") == "true":
        return None
    top = git(path, "rev-parse", "--show-toplevel")
    return canonical(top) if top else None


def resolve_base(root):
    head = git(root, "symbolic-ref", "-q", "refs/remotes/origin/HEAD")
    if head and head.startswith("refs/remotes/origin/"):
        return head[len("refs/remotes/origin/"):]
    for b in BASE_CANDIDATES:
        if git(root, "rev-parse", "-q", "--verify", "refs/heads/" + b) or git(
            root, "rev-parse", "-q", "--verify", "refs/remotes/origin/" + b
        ):
            return b
    return None


# ---------------------------------------------------------------------------
# Store


def read_json(path):
    try:
        with open(path) as f:
            return json.load(f)
    except (OSError, ValueError):
        return None


def load_envs(config):
    """Environments in display order (env files missing from the index last)."""
    index = read_json(os.path.join(config, "environments.json")) or {}
    order = list(index.get("order", []))
    if os.path.isdir(config):
        extra = sorted(
            n[4:-5]
            for n in os.listdir(config)
            if n.startswith("env-") and n.endswith(".json") and n[4:-5] not in order
        )
        order += extra
    envs = []
    for env_id in order:
        path = os.path.join(config, "env-%s.json" % env_id)
        env = read_json(path) or read_json(os.path.join(config, "env-%s.json.bak" % env_id))
        if env:
            envs.append(env)
    return envs


def owners(envs):
    """Real repo path -> environment name."""
    out = {}
    for env in envs:
        for r in env.get("repos", []):
            out[canonical(r["path"])] = env["name"]
    return out


@contextlib.contextmanager
def store_lock(config):
    os.makedirs(config, exist_ok=True)
    with open(os.path.join(config, ".lock"), "a") as f:
        fcntl.flock(f, fcntl.LOCK_EX)
        try:
            yield
        finally:
            fcntl.flock(f, fcntl.LOCK_UN)


def env_path(config, env_id):
    return os.path.join(config, "env-%s.json" % env_id)


def find_env(envs, name):
    hits = [e for e in envs if e["name"].lower() == name.strip().lower()]
    if not hits:
        fail('No environment named "%s". Existing: %s' % (name, ", ".join(e["name"] for e in envs) or "none"))
    return hits[0]


def check_writable(env):
    if env.get("version", 0) > VERSION:
        fail('"%s" was saved by a newer git-desktools; update the app to change it.' % env["name"])
    if env.get("journal"):
        fail('A feature switch in "%s" didn\'t finish. Retry or roll it back in the app first.' % env["name"])


def write_atomic(path, value):
    tmp = path[: -len(".json")] + ".json.tmp"
    with open(tmp, "w") as f:
        json.dump(value, f, indent=2)
    if os.path.exists(path):
        with open(path) as src, open(path[: -len(".json")] + ".json.bak", "w") as dst:
            dst.write(src.read())
    os.replace(tmp, path)


# ---------------------------------------------------------------------------
# Commands


def scan(root, max_depth, taken):
    """Repo roots under root. Doesn't descend into a repo, hidden folders,
    build/dependency folders or symlinks. max_depth 0 = unlimited."""
    hits, seen = [], set()

    def walk(d, depth):
        real = canonical(d)
        if real in seen:
            return
        seen.add(real)
        if os.path.exists(os.path.join(real, ".git")):
            hits.append(real)
            return
        if max_depth and depth >= max_depth:
            return
        try:
            entries = sorted(os.scandir(real), key=lambda e: e.name)
        except OSError:
            return
        for e in entries:
            if e.is_dir(follow_symlinks=False) and not e.name.startswith(".") and e.name not in SKIP:
                walk(e.path, depth + 1)

    walk(root, 0)
    out = []
    for h in hits:
        top = repo_root(h)
        if not top:
            continue
        out.append(
            {
                "root": top,
                "name": os.path.basename(top),
                "relative": os.path.relpath(top, canonical(root)),
                "branch": git(top, "rev-parse", "--abbrev-ref", "HEAD"),
                "baseBranch": resolve_base(top),
                "inEnv": taken.get(top),
            }
        )
    return out


def create(config, name, repos, shared_base, dry_run):
    if dry_run:
        return create_locked(config, name, repos, shared_base, dry_run)
    with store_lock(config):
        create_locked(config, name, repos, shared_base, dry_run)


def create_locked(config, name, repos, shared_base, dry_run):
    name = (name or "").strip()
    if not name:
        fail("Give the environment a name.")
    envs = load_envs(config)
    if any(e["name"].lower() == name.lower() for e in envs):
        fail('An environment named "%s" already exists.' % name)
    taken = owners(envs)
    out_repos, seen, problems = [], set(), []
    for item in repos:
        item = {"path": item} if isinstance(item, str) else dict(item)
        top = repo_root(canonical(item["path"]))
        if not top:
            problems.append("%s is not a git repository with a working folder." % item["path"])
            continue
        if top in seen:
            continue
        seen.add(top)
        if top in taken:
            problems.append('%s already belongs to the "%s" environment.' % (os.path.basename(top), taken[top]))
            continue
        base = (item.get("baseBranch") or "").strip() or None
        out_repos.append(
            {
                "id": new_id("r"),
                "path": top,
                "label": (item.get("label") or "").strip() or os.path.basename(top),
                "baseBranch": base,
            }
        )
    if problems:
        fail("\n".join(problems))
    if not out_repos:
        fail("No repositories to add.")
    env = {
        "version": VERSION,
        "id": new_id("e"),
        "name": name,
        "settings": {"sharedBase": (shared_base or "").strip() or None},
        "repos": out_repos,
        "activeFeatureId": None,
        "features": [],
        "journal": None,
    }
    if not dry_run:
        os.makedirs(config, exist_ok=True)
        write_atomic(os.path.join(config, "env-%s.json" % env["id"]), env)
        index_path = os.path.join(config, "environments.json")
        index = read_json(index_path) or {}
        order = list(index.get("order", []))
        order.append(env["id"])
        write_atomic(index_path, {"version": VERSION, "order": order})
    print(json.dumps({"dryRun": dry_run, "config": config, "env": env}, indent=2))


def find_repo(config, name, search, max_depth):
    """Repos called `name` (folder name or label, case-insensitive): those in
    environments first, then any found under the search folders."""
    want = name.strip().lower()
    envs = load_envs(config)
    taken = owners(envs)
    hits = {}
    for env in envs:
        for r in env.get("repos", []):
            path = canonical(r["path"])
            if want in (os.path.basename(path).lower(), (r.get("label") or "").lower()):
                hits[path] = {"root": path, "name": os.path.basename(path), "label": r.get("label"), "inEnv": env["name"],
                              "exists": os.path.isdir(path)}
    for folder in search:
        folder = canonical(folder)
        if not os.path.isdir(folder):
            continue
        for h in scan(folder, max_depth, taken):
            if h["name"].lower() == want and h["root"] not in hits:
                hits[h["root"]] = {"root": h["root"], "name": h["name"], "label": None, "inEnv": h["inEnv"], "exists": True}
    return list(hits.values())


def feature_work(env, repo_entry, root):
    """Why the repo can't leave `env`: features there that touched it."""
    reasons = []
    for f in env.get("features", []):
        branch, fid = f["branch"], f["id"]
        parts = []
        if repo_entry["id"] in f.get("repos", {}):
            parts.append("recorded in the feature")
        if git(root, "rev-parse", "-q", "--verify", "refs/smart-features/%s/stash" % fid):
            parts.append("parked changes")
        if git(root, "rev-parse", "-q", "--verify", "refs/heads/" + branch):
            parts.append("branch %s" % branch)
        if parts:
            active = " (active)" if env.get("activeFeatureId") == fid else ""
            reasons.append('"%s"%s: %s' % (f["name"], active, ", ".join(parts)))
    return reasons


def add_repo(config, env_name, repo_path, move, dry_run):
    with (contextlib.nullcontext() if dry_run else store_lock(config)):
        envs = load_envs(config)
        target = find_env(envs, env_name)
        check_writable(target)
        root = repo_root(canonical(repo_path))
        if not root:
            fail("%s is not a git repository with a working folder." % repo_path)
        name = os.path.basename(root)
        if any(canonical(r["path"]) == root for r in target.get("repos", [])):
            print(json.dumps({"dryRun": dry_run, "unchanged": True, "message": '%s is already in "%s".' % (name, target["name"])}))
            return
        source = next((e for e in envs if any(canonical(r["path"]) == root for r in e.get("repos", []))), None)
        entry = None
        if source:
            entry = next(r for r in source["repos"] if canonical(r["path"]) == root)
            if not move:
                fail('%s is in "%s". A repo can only be in one environment; move it with --move.' % (name, source["name"]))
            check_writable(source)
            reasons = feature_work(source, entry, root)
            if reasons:
                fail(
                    '%s has feature work in "%s", so it can\'t move yet:\n%s\n'
                    "Finish those features (or switch away and finish them) in the app first."
                    % (name, source["name"], "\n".join(reasons))
                )
        moved = dict(entry) if entry else {"id": new_id("r"), "path": root, "label": name, "baseBranch": None}
        moved["path"] = root

        # Notes about what happens next in the target.
        notes = []
        branch = git(root, "rev-parse", "--abbrev-ref", "HEAD")
        base = moved.get("baseBranch") or (target.get("settings") or {}).get("sharedBase") or resolve_base(root)
        active = next((f for f in target.get("features", []) if f["id"] == target.get("activeFeatureId")), None)
        dirty = bool(git(root, "status", "--porcelain"))
        if branch != base:
            notes.append("%s is on %s, not its base %s; the panel will flag it." % (name, branch, base))
        elif active and dirty:
            notes.append('%s has uncommitted changes; they move onto "%s" (%s) right away.' % (name, active["name"], active["branch"]))

        if not dry_run:
            # Out of the old environment first: a crash in between leaves it in neither, never in both.
            if source:
                source["repos"] = [r for r in source["repos"] if r is not entry]
                write_atomic(env_path(config, source["id"]), source)
            target.setdefault("repos", []).append(moved)
            write_atomic(env_path(config, target["id"]), target)
        print(json.dumps({
            "dryRun": dry_run,
            "repo": moved,
            "from": source["name"] if source else None,
            "to": target["name"],
            "notes": notes,
        }, indent=2))


def main():
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("--config-dir", default=os.environ.get("GDT_SMART_CONFIG", DEFAULT_CONFIG))
    sub = p.add_subparsers(dest="cmd", required=True)
    sub.add_parser("list")
    s = sub.add_parser("scan")
    s.add_argument("folder")
    s.add_argument("--max-depth", type=int, default=3)
    c = sub.add_parser("create")
    c.add_argument("--name", required=True)
    c.add_argument("--repos", required=True, help="JSON file, or - for stdin")
    c.add_argument("--shared-base")
    c.add_argument("--dry-run", action="store_true")
    f = sub.add_parser("find-repo")
    f.add_argument("name")
    f.add_argument("--search", action="append", default=[])
    f.add_argument("--max-depth", type=int, default=4)
    m = sub.add_parser("add-repo")
    m.add_argument("--env", required=True)
    m.add_argument("--repo", required=True)
    m.add_argument("--move", action="store_true")
    m.add_argument("--dry-run", action="store_true")
    a = p.parse_args()
    config = os.path.expanduser(a.config_dir)

    if a.cmd == "list":
        envs = load_envs(config)
        print(
            json.dumps(
                [
                    {
                        "name": e["name"],
                        "id": e["id"],
                        "activeFeature": next(
                            (f["name"] for f in e.get("features", []) if f["id"] == e.get("activeFeatureId")), None
                        ),
                        "repos": [{"label": r.get("label"), "path": r["path"]} for r in e.get("repos", [])],
                    }
                    for e in envs
                ],
                indent=2,
            )
        )
    elif a.cmd == "scan":
        folder = canonical(a.folder)
        if not os.path.isdir(folder):
            fail("%s is not a folder." % a.folder)
        print(json.dumps(scan(folder, a.max_depth, owners(load_envs(config))), indent=2))
    elif a.cmd == "find-repo":
        print(json.dumps(find_repo(config, a.name, a.search, a.max_depth), indent=2))
    elif a.cmd == "add-repo":
        add_repo(config, a.env, a.repo, a.move, a.dry_run)
    elif a.cmd == "create":
        text = sys.stdin.read() if a.repos == "-" else open(a.repos).read()
        create(config, a.name, json.loads(text), a.shared_base, a.dry_run)


if __name__ == "__main__":
    main()
