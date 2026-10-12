import { useCallback, useEffect, useRef, useState } from "react";
import { api, errorText, onSmartEvents } from "../api";
import { showToast } from "../toast";
import { notifyReposChanged } from "../util";
import type { Dot, SmartEnv, SmartFeature, SmartFeaturesUi, SmartRepoStatus, SwitchResult } from "../types";
import { EnvironmentDialog } from "./EnvironmentDialog";
import { FinishFeatureDialog } from "./FinishFeatureDialog";
import { NewFeatureDialog } from "./NewFeatureDialog";
import { SwitchFeatureDialog } from "./SwitchFeatureDialog";
import { BranchIcon, ChevronIcon, FetchIcon, MoreIcon, PlusIcon, PushIcon, RefreshIcon, WarningIcon } from "./icons";

interface Props {
  ui: SmartFeaturesUi;
  onUi: (patch: Partial<SmartFeaturesUi>) => void;
  openPaths: string[];
  onOpenRepo: (path: string) => void;
}

const REFRESH_MS = 30_000;
const DOT_RANK: Record<Dot, number> = { none: 0, green: 1, yellow: 2, red: 3 };

const isQuiet = (s: SmartRepoStatus) => s.dot === "green" && !s.warning;

function worst(statuses: SmartRepoStatus[]): Dot {
  return statuses.reduce<Dot>((w, s) => (DOT_RANK[s.dot] > DOT_RANK[w] ? s.dot : w), "none");
}

/** A feature's footprint across the environment, from branch refs (no checkout). */
function featureSummary(f: SmartFeature, st: SmartRepoStatus[] | undefined) {
  const refs = (st ?? []).flatMap((s) => s.features.filter((x) => x.featureId === f.id).map((x) => ({ s, x })));
  const local = refs.filter(({ x }) => x.local);
  return {
    repos: Object.keys(f.repos).length,
    unpushed: local.reduce((n, { x }) => n + x.unpushed, 0),
    parked: refs.filter(({ x }) => x.parked).length,
    /** Checked out, but its parked changes are still pinned: the restore conflicted. */
    unresolved: refs.filter(({ s, x }) => x.parked && s.branch === f.branch).map(({ s }) => s),
    /** Repos where only origin/<branch> exists (a teammate's work). */
    remoteOnly: refs.filter(({ s, x }) => x.remote && !x.local && !(s.repoId in f.repos)).map(({ s }) => s),
  };
}

const plural = (n: number, one: string, many = `${one}s`) => `${n} ${n === 1 ? one : many}`;

function dotTitle(s: SmartRepoStatus): string {
  if (s.missing) return "Folder not found";
  switch (s.dot) {
    case "red":
      return "Uncommitted changes";
    case "yellow":
      return "Committed locally, not pushed";
    case "green":
      return "No changes, nothing to push";
    default:
      return "";
  }
}

export function SmartFeaturesPanel({ ui, onUi, openPaths, onOpenRepo }: Props) {
  const [envs, setEnvs] = useState<SmartEnv[] | null>(null);
  const [statuses, setStatuses] = useState<Record<string, SmartRepoStatus[]>>({});
  const [loading, setLoading] = useState<Set<string>>(new Set());
  const [fetching, setFetching] = useState<Set<string>>(new Set());
  const [showClean, setShowClean] = useState<Set<string>>(new Set());
  const [editing, setEditing] = useState<SmartEnv | "new" | null>(null);
  const [menu, setMenu] = useState<string | null>(null);
  const [deleting, setDeleting] = useState<SmartEnv | null>(null);
  const [starting, setStarting] = useState<SmartEnv | null>(null);
  const [renaming, setRenaming] = useState<{ env: SmartEnv; feature: SmartFeature; name: string } | null>(null);
  const [working, setWorking] = useState<Set<string>>(new Set());
  const [switching, setSwitching] = useState<{ env: SmartEnv; target: SmartFeature | null } | null>(null);
  const [finishing, setFinishing] = useState<{ env: SmartEnv; feature: SmartFeature } | null>(null);
  const [featureMenu, setFeatureMenu] = useState<string | null>(null);
  const envsRef = useRef(envs);
  envsRef.current = envs;

  const loadEnvs = useCallback(async () => {
    try {
      const { envs, warnings } = await api.sfListEnvs();
      setEnvs(envs);
      for (const w of warnings) showToast(w, "error");
      return envs;
    } catch (e) {
      showToast(errorText(e), "error");
      setEnvs([]);
      return [];
    }
  }, []);

  const refreshEnv = useCallback(async (id: string) => {
    setLoading((s) => new Set(s).add(id));
    try {
      const st = await api.sfStatus(id);
      setStatuses((m) => ({ ...m, [id]: st }));
    } catch {
      /* the environment was deleted meanwhile; the next list load drops it */
    } finally {
      setLoading((s) => {
        const n = new Set(s);
        n.delete(id);
        return n;
      });
    }
  }, []);

  const collapsed = new Set(ui.collapsed);
  const collapsedKey = ui.collapsed.join("\n");

  const refreshAll = useCallback(
    async (list?: SmartEnv[]) => {
      const all = list ?? envsRef.current ?? [];
      const closed = new Set(collapsedKey ? collapsedKey.split("\n") : []);
      await Promise.all(all.filter((e) => !closed.has(e.id)).map((e) => refreshEnv(e.id)));
    },
    [refreshEnv, collapsedKey],
  );

  useEffect(() => {
    void loadEnvs().then((list) => refreshAll(list));
  }, []);

  // Refresh on window focus and on a timer while the panel is mounted (it only
  // mounts while open). Git changes from terminals and editors land this way.
  useEffect(() => {
    const onFocus = () => void refreshAll();
    window.addEventListener("focus", onFocus);
    const t = window.setInterval(() => void refreshAll(), REFRESH_MS);
    return () => {
      window.removeEventListener("focus", onFocus);
      window.clearInterval(t);
    };
  }, [refreshAll]);

  // Backend watcher events: repos changing on disk, and repos moved onto the
  // active feature's branch. Refreshes are coalesced per environment.
  useEffect(() => {
    const timers = new Map<string, number>();
    const soon = (envId: string, reload = false) => {
      window.clearTimeout(timers.get(envId));
      timers.set(
        envId,
        window.setTimeout(() => {
          timers.delete(envId);
          if (reload) void loadEnvs().then(() => refreshEnv(envId));
          else void refreshEnv(envId);
        }, 400),
      );
    };
    const off = onSmartEvents({
      repoChanged: ({ envId }) => soon(envId),
      autoBranched: (b) => {
        if (b.created) showToast(`${b.label} moved to ${b.branch}`, "success");
        notifyReposChanged();
        soon(b.envId, true);
      },
      autoBranchFailed: ({ error }) => showToast(error, "error"),
    });
    return () => {
      off();
      timers.forEach((t) => window.clearTimeout(t));
    };
  }, [loadEnvs, refreshEnv]);

  /** Run an environment-level action with a busy flag, then reload. */
  const withEnv = async (env: SmartEnv, fn: () => Promise<void>) => {
    setMenu(null);
    setWorking((s) => new Set(s).add(env.id));
    try {
      await fn();
    } catch (e) {
      showToast(errorText(e), "error");
    } finally {
      setWorking((s) => {
        const n = new Set(s);
        n.delete(env.id);
        return n;
      });
      notifyReposChanged();
      await loadEnvs();
      void refreshEnv(env.id);
    }
  };

  const backToBase = (env: SmartEnv) => {
    setMenu(null);
    setSwitching({ env, target: null });
  };

  /** After a switch, Back to base or Finish dialog ran (result null = it failed). */
  const afterRun = (env: SmartEnv, result: SwitchResult | null, success?: string) => {
    if (result) {
      if (success) showToast(success, "success");
      for (const w of result.warnings) showToast(w, "error");
    }
    void withEnv(env, async () => {});
  };

  const resumeSwitch = (env: SmartEnv, mode: "retry" | "rollback") =>
    withEnv(env, async () => {
      const r = await api.sfResumeSwitch(env.id, mode);
      const f = r.env.features.find((x) => x.id === r.env.activeFeatureId);
      showToast(
        mode === "retry"
          ? `Switch finished: ${f ? `“${f.name}”` : "base"} is active`
          : `Rolled back to ${f ? `“${f.name}”` : "base"}`,
        "success",
      );
      for (const w of r.warnings) showToast(w, "error");
    });

  const markResolved = (env: SmartEnv, f: SmartFeature, s: SmartRepoStatus) =>
    withEnv(env, async () => {
      await api.sfMarkResolved(env.id, f.id, s.repoId);
      showToast(`${s.label}: parked changes marked resolved`, "success");
    });

  const journalBanner = (env: SmartEnv) => {
    const j = env.journal;
    if (!j) return null;
    const name = (id: string | null) => (id ? `“${env.features.find((f) => f.id === id)?.name ?? "?"}”` : "base");
    const failed = j.steps.find((s) => s.status === "failed");
    const busy = working.has(env.id);
    return (
      <div className="sf-journal">
        <div>
          <WarningIcon size={12} /> The switch from {name(j.from)} to {name(j.to)} didn't finish.
        </div>
        {failed && (
          <div className="dim small mono sf-journal-error">
            {failed.label}: {failed.message}
          </div>
        )}
        <div className="sf-journal-actions">
          <button className="btn small" disabled={busy} onClick={() => void resumeSwitch(env, "rollback")}>
            Roll back to {name(j.from)}
          </button>
          <button className="btn small primary" disabled={busy} onClick={() => void resumeSwitch(env, "retry")}>
            Retry
          </button>
        </div>
      </div>
    );
  };

  const pushAll = (env: SmartEnv, f: SmartFeature) =>
    withEnv(env, async () => {
      const out = await api.sfPushAll(env.id, f.id);
      const pushed = out.filter((o) => o.status === "pushed");
      const failed = out.filter((o) => o.status === "failed");
      if (failed.length)
        showToast(
          `Push failed for ${failed.length} of ${out.length}:\n${failed.map((o) => `${o.label}: ${o.detail.split("\n").pop()}`).join("\n")}`,
          "error",
        );
      else if (pushed.length) showToast(`Pushed ${f.branch} in ${pushed.map((o) => o.label).join(", ")}`, "success");
      else showToast(`Nothing to push for ${f.name}`, "success");
    });

  const checkoutRemote = (env: SmartEnv, s: SmartRepoStatus, f: SmartFeature) =>
    withEnv(env, async () => {
      await api.sfCheckoutRemote(env.id, s.repoId);
      showToast(`${s.label} is on ${f.branch} from origin`, "success");
    });

  const saveRename = async () => {
    if (!renaming) return;
    const { env, feature, name } = renaming;
    setRenaming(null);
    await withEnv(env, async () => {
      await api.sfRenameFeature(env.id, feature.id, name);
    });
  };

  /** After starting: optionally fast-forward every clean repo still on base. */
  const pullBases = async (env: SmartEnv) => {
    const st = await api.sfStatus(env.id);
    const targets = st.filter((s) => s.branch && s.branch === s.baseBranch && s.hasUpstream && s.dot !== "red");
    const failed: string[] = [];
    const queue = [...targets];
    const worker = async () => {
      for (let s = queue.shift(); s; s = queue.shift()) {
        try {
          await api.pullRepo(s.path);
        } catch (e) {
          failed.push(`${s.label}: ${errorText(e).split("\n")[0]}`);
        }
      }
    };
    await Promise.all([worker(), worker(), worker(), worker()]);
    if (failed.length)
      showToast(
        `Couldn't pull ${failed.length} base ${failed.length === 1 ? "branch" : "branches"}:\n${failed.join("\n")}`,
        "error",
      );
  };

  const toggleEnv = (id: string) => {
    const next = new Set(collapsed);
    if (next.has(id)) {
      next.delete(id);
      void refreshEnv(id);
    } else next.add(id);
    onUi({ collapsed: [...next] });
  };

  const fetchAll = async (env: SmartEnv) => {
    setMenu(null);
    setFetching((s) => new Set(s).add(env.id));
    const failed: string[] = [];
    // A few at a time, so a big environment doesn't open dozens of connections.
    const queue = [...env.repos];
    const worker = async () => {
      for (let r = queue.shift(); r; r = queue.shift()) {
        try {
          await api.fetchRepo(r.path, false);
        } catch (e) {
          failed.push(`${r.label}: ${errorText(e).split("\n").pop()}`);
        }
      }
    };
    await Promise.all([worker(), worker(), worker(), worker()]);
    setFetching((s) => {
      const n = new Set(s);
      n.delete(env.id);
      return n;
    });
    if (failed.length)
      showToast(`Fetch failed for ${failed.length} of ${env.repos.length}:\n${failed.join("\n")}`, "error");
    else
      showToast(
        `Fetched ${env.repos.length} ${env.repos.length === 1 ? "repository" : "repositories"} in ${env.name}`,
        "success",
      );
    void refreshEnv(env.id);
  };

  const confirmDelete = async () => {
    if (!deleting) return;
    try {
      await api.sfDeleteEnv(deleting.id);
      setStatuses((m) => {
        const n = { ...m };
        delete n[deleting.id];
        return n;
      });
      showToast(`Removed environment ${deleting.name}`, "success");
      await loadEnvs();
    } catch (e) {
      showToast(errorText(e), "error");
    }
    setDeleting(null);
  };

  const repoRow = (s: SmartRepoStatus) => {
    const offBranch = !!s.branch && s.branch !== s.expectedBranch;
    return (
      <button
        key={s.repoId || s.path}
        className={`sf-repo${s.missing ? " missing" : ""}`}
        onClick={() => !s.missing && onOpenRepo(s.path)}
        title={[s.path, s.warning].filter(Boolean).join("\n")}
      >
        <span className={`sf-dot ${s.dot}`} title={dotTitle(s)} />
        <span className="sf-repo-name">{s.label}</span>
        {s.warning && <WarningIcon size={12} className="sf-warn" />}
        <span className="sf-counts">
          {s.conflicted > 0 && <span className="sf-c conflict">!{s.conflicted}</span>}
          {s.staged + s.unstaged > 0 && <span className="sf-c mod">✎{s.staged + s.unstaged}</span>}
          {s.untracked > 0 && <span className="sf-c add">✚{s.untracked}</span>}
          {s.hasUpstream && s.ahead > 0 && <span className="sf-c">↑{s.ahead}</span>}
          {!s.hasUpstream && s.aheadOfBase > 0 && (
            <span className="sf-c" title="Commits ahead of base, branch not pushed yet">
              ↑{s.aheadOfBase}*
            </span>
          )}
          {s.behind > 0 && <span className="sf-c">↓{s.behind}</span>}
        </span>
        <span className={`sf-branch${offBranch ? " off" : ""}`}>
          {s.missing ? "not found" : s.detached ? "detached" : s.branch}
        </span>
      </button>
    );
  };

  const activeOf = (env: SmartEnv) => env.features.find((f) => f.id === env.activeFeatureId) ?? null;

  const featureBar = (env: SmartEnv, st: SmartRepoStatus[] | undefined) => {
    const f = activeOf(env);
    const busy = working.has(env.id);
    if (!f)
      return (
        <div className="sf-feature-bar none">
          <span className="dim">No active feature</span>
          <button className="btn small" onClick={() => setStarting(env)} disabled={busy}>
            <PlusIcon size={11} /> Feature
          </button>
        </div>
      );
    const sum = featureSummary(f, st);
    return (
      <>
        <div className="sf-feature-bar" title={`${f.name}\nbranch ${f.branch}`}>
          <span className="sf-star">★</span>
          <span className="sf-feature-name">{f.name}</span>
          {f.name !== f.branch && <span className="sf-feature-branch">{f.branch}</span>}
          <span className="dim small sf-feature-sum">
            {sum.repos ? plural(sum.repos, "repo") : "no changes yet"}
            {sum.unpushed > 0 && ` · ↑${sum.unpushed}`}
          </span>
          <button
            className="icon-btn"
            title={sum.unpushed ? `Push ${f.branch} in every repo with unpushed commits` : "Nothing to push"}
            disabled={busy || sum.unpushed === 0}
            onClick={() => void pushAll(env, f)}
          >
            <PushIcon size={13} className={busy ? "spin-slow" : undefined} />
          </button>
        </div>
        {sum.unresolved.map((s) => (
          <div key={s.repoId} className="sf-remote-only">
            <WarningIcon size={12} className="sf-warn" />
            <span className="sf-repo-name">{s.label}</span>
            <span className="dim small">parked changes conflicted</span>
            <button
              className="btn small"
              disabled={busy || s.conflicted > 0}
              title={
                s.conflicted > 0
                  ? "Resolve the conflicts and stage the files first"
                  : "The parked changes are applied; forget the parked copy"
              }
              onClick={() => void markResolved(env, f, s)}
            >
              Mark resolved
            </button>
          </div>
        ))}
        {sum.remoteOnly.map((s) => (
          <div key={s.repoId} className="sf-remote-only">
            <span className="sf-repo-name">{s.label}</span>
            <span className="dim small">has origin/{f.branch}</span>
            <button
              className="btn small"
              disabled={busy || s.dot === "red" || s.branch !== s.baseBranch}
              title={
                s.dot === "red"
                  ? "Commit or discard its changes first"
                  : `Create a local ${f.branch} tracking origin/${f.branch}`
              }
              onClick={() => void checkoutRemote(env, s, f)}
            >
              Check out
            </button>
          </div>
        ))}
      </>
    );
  };

  const otherFeatures = (env: SmartEnv, st: SmartRepoStatus[] | undefined) => {
    const others = env.features.filter((f) => f.id !== env.activeFeatureId);
    if (!others.length) return null;
    return (
      <div className="sf-features">
        <div className="sf-features-head">Features</div>
        {others.map((f) => {
          const sum = featureSummary(f, st);
          return (
            <div key={f.id} className="sf-feature-row" title={`branch ${f.branch}`}>
              <span className="sf-feature-name">{f.name}</span>
              <span className="dim small sf-feature-sum">
                {plural(sum.repos, "repo")}
                {sum.unpushed > 0 && ` · ↑${sum.unpushed}`}
                {sum.parked > 0 && (
                  <span title={`Parked changes in ${plural(sum.parked, "repo")}`}> · 📦{sum.parked}</span>
                )}
              </span>
              <button
                className="btn small"
                disabled={working.has(env.id) || !!env.journal}
                onClick={() => setSwitching({ env, target: f })}
              >
                Switch
              </button>
              <button
                className="icon-btn"
                title="More"
                onClick={() => setFeatureMenu(featureMenu === f.id ? null : f.id)}
              >
                <MoreIcon size={13} />
              </button>
              {featureMenu === f.id && (
                <>
                  <div className="backdrop" onMouseDown={() => setFeatureMenu(null)} />
                  <div className="popover sf-menu sf-feature-menu">
                    <button
                      disabled={sum.unpushed === 0}
                      onClick={() => {
                        setFeatureMenu(null);
                        void pushAll(env, f);
                      }}
                    >
                      Push all
                    </button>
                    <button
                      onClick={() => {
                        setFeatureMenu(null);
                        setRenaming({ env, feature: f, name: f.name });
                      }}
                    >
                      Rename…
                    </button>
                    <button
                      className="danger"
                      onClick={() => {
                        setFeatureMenu(null);
                        setFinishing({ env, feature: f });
                      }}
                    >
                      Finish…
                    </button>
                  </div>
                </>
              )}
            </div>
          );
        })}
      </div>
    );
  };

  const envBlock = (env: SmartEnv) => {
    const open = !collapsed.has(env.id);
    const st = statuses[env.id];
    const changed = st?.filter((s) => !isQuiet(s)) ?? [];
    const clean = st?.filter(isQuiet) ?? [];
    const cleanOpen = showClean.has(env.id) && !ui.changedOnly;
    return (
      <div key={env.id} className="sf-env">
        <div className="sf-env-head">
          <button className="sf-env-toggle" onClick={() => toggleEnv(env.id)}>
            <ChevronIcon size={12} open={open} />
            <span className={`sf-dot ${st ? worst(st) : "none"}`} />
            <span className="sf-env-name">{env.name}</span>
          </button>
          <span className="dim small sf-env-sum">
            {st
              ? changed.length
                ? `${changed.length} of ${st.length} need attention`
                : `${st.length} clean`
              : `${env.repos.length} repos`}
          </span>
          <button
            className="icon-btn"
            title="Fetch all repositories"
            disabled={fetching.has(env.id)}
            onClick={() => void fetchAll(env)}
          >
            <FetchIcon size={13} className={fetching.has(env.id) ? "spin-slow" : undefined} />
          </button>
          <button className="icon-btn" title="More" onClick={() => setMenu(menu === env.id ? null : env.id)}>
            <MoreIcon size={14} />
          </button>
          {menu === env.id && (
            <>
              <div className="backdrop" onMouseDown={() => setMenu(null)} />
              <div className="popover sf-menu">
                <button
                  onClick={() => {
                    setMenu(null);
                    setStarting(env);
                  }}
                >
                  New feature…
                </button>
                {activeOf(env) && (
                  <>
                    <button onClick={() => void pushAll(env, activeOf(env)!)}>Push all</button>
                    <button
                      onClick={() => {
                        setMenu(null);
                        const f = activeOf(env)!;
                        setRenaming({ env, feature: f, name: f.name });
                      }}
                    >
                      Rename feature…
                    </button>
                    <button onClick={() => backToBase(env)}>Back to base…</button>
                    <button
                      className="danger"
                      onClick={() => {
                        setMenu(null);
                        setFinishing({ env, feature: activeOf(env)! });
                      }}
                    >
                      Finish feature…
                    </button>
                  </>
                )}
                <div className="sf-menu-sep" />
                <button
                  onClick={() => {
                    setMenu(null);
                    setEditing(env);
                  }}
                >
                  Edit environment…
                </button>
                <button onClick={() => void fetchAll(env)}>Fetch all</button>
                <button
                  className="danger"
                  onClick={() => {
                    setMenu(null);
                    setDeleting(env);
                  }}
                >
                  Delete environment…
                </button>
              </div>
            </>
          )}
        </div>
        {open && (
          <div className="sf-env-body">
            {journalBanner(env)}
            {featureBar(env, st)}
            {!st && loading.has(env.id) && <div className="sf-empty">Reading status…</div>}
            {st && st.length === 0 && (
              <div className="sf-empty">
                No repositories yet.{" "}
                <button className="link-btn" onClick={() => setEditing(env)}>
                  Add some
                </button>
              </div>
            )}
            {changed.map(repoRow)}
            {cleanOpen && clean.map(repoRow)}
            {clean.length > 0 && !ui.changedOnly && (
              <button
                className="sf-clean-toggle"
                onClick={() =>
                  setShowClean((s) => {
                    const n = new Set(s);
                    if (n.has(env.id)) n.delete(env.id);
                    else n.add(env.id);
                    return n;
                  })
                }
              >
                <ChevronIcon size={11} open={cleanOpen} />
                <span className="sf-dot green" />
                {clean.length} clean {clean.length === 1 ? "repo" : "repos"}
              </button>
            )}
            {otherFeatures(env, st)}
          </div>
        )}
      </div>
    );
  };

  return (
    <div className="sf-panel" style={{ width: ui.width }}>
      <div className="sf-head">
        <span className="sf-title">
          <BranchIcon size={13} /> Smart Features
        </span>
        <button className="icon-btn" title="Refresh status" onClick={() => void loadEnvs().then((l) => refreshAll(l))}>
          <RefreshIcon size={13} className={loading.size ? "spin-slow" : undefined} />
        </button>
        <button className="icon-btn" title="New Smart Environment" onClick={() => setEditing("new")}>
          <PlusIcon size={14} />
        </button>
      </div>
      {envs && envs.length > 0 && (
        <label className="sf-filter">
          <input type="checkbox" checked={ui.changedOnly} onChange={(e) => onUi({ changedOnly: e.target.checked })} />
          Changed only
        </label>
      )}
      <div className="sf-list">
        {envs === null && <div className="sf-empty">Loading…</div>}
        {envs?.length === 0 && (
          <div className="sf-intro">
            <p>
              Group the repositories you work on together, like a server and its game client, into a{" "}
              <b>Smart Environment</b> to see their status at a glance.
            </p>
            <p className="dim small">
              <span className="sf-dot red" /> uncommitted changes <span className="sf-dot yellow" /> not pushed{" "}
              <span className="sf-dot green" /> clean
            </p>
            <button className="btn primary" onClick={() => setEditing("new")}>
              <PlusIcon size={13} /> New environment
            </button>
          </div>
        )}
        {envs?.map(envBlock)}
      </div>

      {editing && (
        <EnvironmentDialog
          env={editing === "new" ? null : editing}
          openPaths={openPaths}
          onClose={() => setEditing(null)}
          onSaved={(saved) => {
            setEditing(null);
            showToast(`Saved environment ${saved.name}`, "success");
            void loadEnvs().then(() => refreshEnv(saved.id));
          }}
        />
      )}
      {starting && (
        <NewFeatureDialog
          env={starting}
          onClose={() => setStarting(null)}
          onStarted={(res, pull) => {
            const env = starting;
            setStarting(null);
            const f = res.env.features.find((x) => x.id === res.env.activeFeatureId);
            showToast(
              res.moved.length
                ? `Started ${f?.name}. Moved ${res.moved.map((m) => m.label).join(", ")} to ${f?.branch}.`
                : `Started ${f?.name}. Repos join it as their files change.`,
              "success",
            );
            for (const w of res.warnings) showToast(w, "error");
            void withEnv(env, async () => {
              if (pull) await pullBases(env);
            });
          }}
        />
      )}
      {switching && (
        <SwitchFeatureDialog
          env={switching.env}
          target={switching.target}
          onClose={() => setSwitching(null)}
          onDone={(res) => {
            const { env, target } = switching;
            if (res) setSwitching(null);
            afterRun(env, res, target ? `Switched to “${target.name}”` : `${env.name} is back on base`);
          }}
        />
      )}
      {finishing && (
        <FinishFeatureDialog
          env={finishing.env}
          feature={finishing.feature}
          onClose={() => setFinishing(null)}
          onDone={(res) => {
            const { env, feature } = finishing;
            setFinishing(null);
            afterRun(env, res, `Finished “${feature.name}”`);
          }}
        />
      )}
      {renaming && (
        <div className="modal-backdrop" onMouseDown={() => setRenaming(null)}>
          <div className="modal sf-confirm" onMouseDown={(e) => e.stopPropagation()}>
            <div className="modal-title">Rename feature</div>
            <label className="field">
              <span>Display name (the branch stays {renaming.feature.branch})</span>
              <input
                autoFocus
                value={renaming.name}
                onChange={(e) => setRenaming({ ...renaming, name: e.target.value })}
                onKeyDown={(e) => e.key === "Enter" && void saveRename()}
              />
            </label>
            <div className="popover-actions">
              <button className="btn" onClick={() => setRenaming(null)}>
                Cancel
              </button>
              <button className="btn primary" disabled={!renaming.name.trim()} onClick={() => void saveRename()}>
                Rename
              </button>
            </div>
          </div>
        </div>
      )}
      {deleting && (
        <div className="modal-backdrop" onMouseDown={() => setDeleting(null)}>
          <div className="modal sf-confirm" onMouseDown={(e) => e.stopPropagation()}>
            <div className="modal-title">Delete “{deleting.name}”?</div>
            <p>
              This only forgets the environment. Its {deleting.repos.length}{" "}
              {deleting.repos.length === 1 ? "repository stays" : "repositories stay"} exactly as they are: no files,
              branches or commits are touched.
            </p>
            <div className="popover-actions">
              <button className="btn" onClick={() => setDeleting(null)}>
                Cancel
              </button>
              <button className="btn danger" onClick={() => void confirmDelete()}>
                Delete environment
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
