import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { api, errorText } from "../api";
import { WIP_ID, type FileChange, type FileViewMode, type OpenFile, type RepoData, type WipStatus } from "../types";
import { fileStats, plural, relativeTime } from "../util";
import { showToast } from "../toast";
import { CommitList, type ListHandle } from "./CommitList";
import { CommitPanel } from "./CommitPanel";
import { DiffView } from "./DiffView";
import { ChevronIcon, FetchIcon, PullIcon, PushIcon, RefreshIcon, TerminalIcon, XIcon } from "./icons";
import { clamp, Resizer } from "./Resizer";
import { Sidebar } from "./Sidebar";
import { TerminalPanel, type TermKind, type TermSession } from "./TerminalPanel";
import { emptyDraft, WipPanel, type CommitDraft } from "./WipPanel";

interface Props {
  path: string;
  active: boolean;
  autoFetchSeconds: number;
  /** Delay before this tab's first background fetch, so tabs don't all fetch at once. */
  fetchDelayMs: number;
}

const POLL_MS = 3000;
const BACKOFF_SECONDS = [60, 120, 300, 600];
const EMPTY_WIP: WipStatus = { unstaged: [], staged: [] };

/** Unique changed paths across staged and unstaged, preferring the staged status. */
function wipFiles(w: WipStatus): FileChange[] {
  const map = new Map<string, FileChange>();
  for (const f of w.staged) map.set(f.path, f);
  for (const f of w.unstaged) if (!map.has(f.path)) map.set(f.path, f);
  return [...map.values()];
}

export function RepoView({ path, active, autoFetchSeconds, fetchDelayMs }: Props) {
  const [data, setData] = useState<RepoData | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [wip, setWip] = useState<WipStatus>(EMPTY_WIP);
  const [wipVersion, setWipVersion] = useState(0);
  const [selected, setSelected] = useState<string | null>(null);
  const [openFile, setOpenFile] = useState<OpenFile | null>(null);
  const [viewMode, setViewMode] = useState<FileViewMode>("path");
  const [sidebarOpen, setSidebarOpen] = useState(true);
  const [detailW, setDetailW] = useState(440);
  const [draft, setDraft] = useState<CommitDraft>(emptyDraft);
  const [busy, setBusy] = useState({ fetch: false, pull: false, push: false });
  const [banner, setBanner] = useState<string | null>(null);
  const [fetchInfo, setFetchInfo] = useState<{ last: number | null; error: string | null }>({ last: null, error: null });
  const [pushPrompt, setPushPrompt] = useState<{ remote: string; branch: string } | null>(null);

  const [sessions, setSessions] = useState<TermSession[]>([]);
  const [activeTerm, setActiveTerm] = useState<number | null>(null);
  const [termOpen, setTermOpen] = useState(false);
  const [termH, setTermH] = useState(300);
  const termCounter = useRef({ key: 0, claude: 0, shell: 0 });

  const dataRef = useRef(data);
  dataRef.current = data;
  const wipKeyRef = useRef("");
  const failuresRef = useRef(0);
  const checking = useRef(false);
  const listRef = useRef<ListHandle | null>(null);

  const load = useCallback(async () => {
    try {
      const d = await api.loadRepo(path);
      setData(d);
      setLoadError(null);
      const fallback = d.hasWip ? WIP_ID : (d.head.oid ?? d.commits[0]?.id ?? null);
      setSelected((sel) => {
        if (sel === WIP_ID) return d.hasWip ? sel : fallback;
        if (sel && d.commits.some((c) => c.id === sel)) return sel;
        return fallback;
      });
      return d;
    } catch (e) {
      setLoadError(errorText(e));
      return null;
    }
  }, [path]);

  const refreshWip = useCallback(async () => {
    try {
      const w = await api.wipStatus(path);
      const key = JSON.stringify(w);
      if (key !== wipKeyRef.current) {
        wipKeyRef.current = key;
        setWip(w);
      }
      setWipVersion((v) => v + 1);
      const dirty = w.staged.length + w.unstaged.length > 0;
      if (dataRef.current && dirty !== dataRef.current.hasWip) await load();
    } catch {
      /* repo may be mid-operation (e.g. index.lock); try again next poll */
    }
  }, [path, load]);

  const check = useCallback(async () => {
    if (checking.current) return;
    checking.current = true;
    try {
      const fp = await api.repoFingerprint(path).catch(() => null);
      if (fp !== null && fp !== dataRef.current?.fingerprint) await load();
      await refreshWip();
    } finally {
      checking.current = false;
    }
  }, [path, load, refreshWip]);

  useEffect(() => {
    void load().then(() => refreshWip());
  }, [load, refreshWip]);

  // While visible, keep up with changes made outside the app (or by Claude in the terminal).
  useEffect(() => {
    if (!active) return;
    void check();
    const onFocus = () => void check();
    window.addEventListener("focus", onFocus);
    const id = window.setInterval(() => {
      if (document.visibilityState === "visible") void check();
    }, POLL_MS);
    return () => {
      window.removeEventListener("focus", onFocus);
      window.clearInterval(id);
    };
  }, [active, check]);

  // A WIP file whose diff is open may move between the unstaged and staged lists.
  useEffect(() => {
    if (openFile?.kind !== "wip") return;
    const same = openFile.staged ? wip.staged : wip.unstaged;
    const fresh = same.find((f) => f.path === openFile.file.path);
    if (fresh) {
      if (fresh.status !== openFile.file.status) setOpenFile({ ...openFile, file: fresh });
      return;
    }
    const other = (openFile.staged ? wip.unstaged : wip.staged).find((f) => f.path === openFile.file.path);
    setOpenFile(other ? { kind: "wip", staged: !openFile.staged, file: other } : null);
  }, [wip, openFile]);

  const doFetch = useCallback(
    async (quiet: boolean) => {
      if (!quiet) setBusy((b) => ({ ...b, fetch: true }));
      try {
        const r = await api.fetchRepo(path, quiet);
        if (!r.skipped) {
          failuresRef.current = 0;
          setFetchInfo({ last: Date.now(), error: null });
        } else if (!quiet) {
          showToast("This repository has no remotes to fetch from.");
        }
        await check();
      } catch (e) {
        const msg = errorText(e);
        if (quiet) failuresRef.current++;
        else failuresRef.current = 0;
        setFetchInfo((f) => ({ ...f, error: msg }));
        if (!quiet) setBanner(msg);
      } finally {
        if (!quiet) setBusy((b) => ({ ...b, fetch: false }));
      }
    },
    [path, check],
  );

  const doFetchRef = useRef(doFetch);
  doFetchRef.current = doFetch;

  // Background fetch (never pulls). Runs for every open tab, backing off after failures.
  useEffect(() => {
    if (!autoFetchSeconds || autoFetchSeconds <= 0) return;
    let cancelled = false;
    let timer = 0;
    const tick = async () => {
      if (cancelled) return;
      await doFetchRef.current(true);
      if (cancelled) return;
      const f = failuresRef.current;
      const delay = f === 0 ? autoFetchSeconds : Math.max(autoFetchSeconds, BACKOFF_SECONDS[Math.min(f - 1, 3)]);
      timer = window.setTimeout(tick, delay * 1000);
    };
    timer = window.setTimeout(tick, fetchDelayMs);
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, [autoFetchSeconds, fetchDelayMs]);

  const headLocal = data?.locals.find((l) => l.isHead) ?? null;

  const doPull = async () => {
    setBusy((b) => ({ ...b, pull: true }));
    setBanner(null);
    try {
      const r = await api.pullRepo(path);
      failuresRef.current = 0;
      setFetchInfo({ last: Date.now(), error: null });
      showToast(r.count > 0 ? `Pulled ${plural(r.count, "commit")} from ${r.upstream}` : "Already up to date", "success");
      await load();
      await refreshWip();
    } catch (e) {
      setBanner(errorText(e));
    } finally {
      setBusy((b) => ({ ...b, pull: false }));
    }
  };

  const doPush = async (remote: string | null, branch: string | null) => {
    setPushPrompt(null);
    setBusy((b) => ({ ...b, push: true }));
    setBanner(null);
    const ahead = headLocal?.ahead ?? 0;
    try {
      const r = await api.pushRepo(path, remote, branch);
      const target = r.upstream ?? (remote && branch ? `${remote}/${branch}` : "remote");
      showToast(
        remote ? `Pushed ${branch} to ${target}` : ahead > 0 ? `Pushed ${plural(ahead, "commit")} to ${target}` : "Everything up-to-date",
        "success",
      );
      await load();
    } catch (e) {
      const msg = errorText(e);
      setBanner(
        /rejected|fetch first|non-fast-forward/i.test(msg)
          ? `${msg}\n\nThe remote has commits you don't have yet. Pull first, then push again.`
          : msg,
      );
    } finally {
      setBusy((b) => ({ ...b, push: false }));
    }
  };

  const startPush = () => {
    if (!data?.head.branch) return;
    if (headLocal?.upstream) {
      void doPush(null, null);
      return;
    }
    if (data.remoteInfos.length === 0) {
      setBanner("This repository has no remotes to push to. Add one in the terminal: git remote add origin <url>");
      return;
    }
    const remote = data.remoteInfos.find((r) => r.name === "origin")?.name ?? data.remoteInfos[0].name;
    setPushPrompt({ remote, branch: data.head.branch });
  };

  const addSession = (kind: TermKind) => {
    const c = termCounter.current;
    c.key++;
    c[kind]++;
    const s: TermSession = { key: c.key, kind, title: kind === "claude" ? `Claude ${c.claude}` : `Shell ${c.shell}` };
    setSessions((ss) => [...ss, s]);
    setActiveTerm(s.key);
    setTermOpen(true);
  };
  const closeSession = (key: number) => {
    const next = sessions.filter((s) => s.key !== key);
    setSessions(next);
    if (activeTerm === key) setActiveTerm(next[next.length - 1]?.key ?? null);
    if (next.length === 0) setTermOpen(false);
  };
  const toggleTerminal = () => {
    if (sessions.length === 0) addSession("claude");
    else setTermOpen((o) => !o);
  };
  const toggleRef = useRef(toggleTerminal);
  toggleRef.current = toggleTerminal;

  useEffect(() => {
    if (!active) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.ctrlKey && (e.key === "`" || e.code === "Backquote")) {
        e.preventDefault();
        e.stopPropagation();
        toggleRef.current();
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [active]);

  const select = useCallback((id: string) => {
    setSelected(id);
    setOpenFile((f) => {
      if (!f) return f;
      if (f.kind === "commit" && f.commit === id) return f;
      if (f.kind === "wip" && id === WIP_ID) return f;
      return null;
    });
  }, []);

  const jump = (oid: string) => {
    if (!data?.commits.some((c) => c.id === oid)) {
      showToast("That commit isn't in the loaded history.");
      return;
    }
    select(oid);
    listRef.current?.scrollToId(oid);
  };

  const closeFile = useCallback(() => setOpenFile(null), []);
  const allWipFiles = useMemo(() => wipFiles(wip), [wip]);
  const wipStats = useMemo(() => (data?.hasWip ? fileStats(allWipFiles) : null), [data?.hasWip, allWipFiles]);

  if (!data) {
    return (
      <div className="repo-view">
        <div className="center-msg">{loadError ? <div className="panel-error">{loadError}</div> : "Loading repository…"}</div>
      </div>
    );
  }

  const { head } = data;
  const hasUpstream = !!headLocal?.upstream;
  const ahead = headLocal?.ahead ?? 0;
  const behind = headLocal?.behind ?? 0;
  const pullTitle = head.detached
    ? "Pull is unavailable on a detached HEAD"
    : !hasUpstream
      ? "This branch has no upstream branch to pull from"
      : behind > 0
        ? `Fast-forward ${head.branch} by ${plural(behind, "commit")} from ${headLocal?.upstream}`
        : `Pull from ${headLocal?.upstream} (fast-forward only)`;
  const pushTitle = head.detached
    ? "Push is unavailable on a detached HEAD"
    : head.unborn
      ? "Make a commit before pushing"
      : !hasUpstream
        ? `Push ${head.branch} and set its upstream`
        : ahead > 0
          ? `Push ${plural(ahead, "commit")} to ${headLocal?.upstream}`
          : `Nothing to push — ${headLocal?.upstream} is up to date`;
  const fetchTitle = fetchInfo.error
    ? `Last fetch failed:\n${fetchInfo.error}`
    : fetchInfo.last
      ? `Fetch all remotes — last fetched ${relativeTime(fetchInfo.last)}`
      : "Fetch all remotes";

  return (
    <div className="repo-view">
      <div className="toolbar">
        <div className="tb-field">
          <div className="tb-label">repository</div>
          <div className="tb-value">{data.name}</div>
        </div>
        <span className="tb-sep">›</span>
        <div className="tb-field">
          <div className="tb-label">branch</div>
          <div className="tb-value">{head.branch ?? (head.detached ? "detached HEAD" : "—")}</div>
        </div>
        <div className="tb-actions">
          <ToolButton icon={<FetchIcon size={18} />} label="Fetch" busy={busy.fetch} dot={!!fetchInfo.error} title={fetchTitle} onClick={() => void doFetch(false)} />
          <ToolButton
            icon={<PullIcon size={18} />}
            label="Pull"
            busy={busy.pull}
            badge={behind > 0 ? `${behind}↓` : null}
            disabled={head.detached || !hasUpstream}
            title={pullTitle}
            onClick={() => void doPull()}
          />
          <div className="tb-anchor">
            <ToolButton
              icon={<PushIcon size={18} />}
              label="Push"
              busy={busy.push}
              badge={ahead > 0 ? `${ahead}↑` : null}
              dim={hasUpstream && ahead === 0}
              disabled={head.detached || head.unborn || !head.branch}
              title={pushTitle}
              onClick={startPush}
            />
            {pushPrompt && (
              <PushPrompt
                remotes={data.remoteInfos.map((r) => r.name)}
                initial={pushPrompt}
                onCancel={() => setPushPrompt(null)}
                onPush={(r, b) => void doPush(r, b)}
              />
            )}
          </div>
          <ToolButton
            icon={<RefreshIcon size={18} />}
            label="Refresh"
            title="Reload the graph and working-tree status"
            onClick={() => void load().then(() => refreshWip())}
          />
          <div className="tb-divider" />
          <ToolButton icon={<TerminalIcon size={18} />} label="Terminal" active={termOpen} title="Open a Claude terminal in this repo (Ctrl+`)" onClick={toggleTerminal} />
        </div>
      </div>
      {banner && (
        <div className="banner">
          <pre>{banner}</pre>
          <button className="icon-btn" onClick={() => setBanner(null)} title="Dismiss">
            <XIcon size={14} />
          </button>
        </div>
      )}
      <div className="repo-main">
        {sidebarOpen ? (
          <Sidebar data={data} onJump={jump} onCollapse={() => setSidebarOpen(false)} />
        ) : (
          <div className="sb-collapsed">
            <button className="icon-btn round" onClick={() => setSidebarOpen(true)} title="Show sidebar">
              <ChevronIcon size={14} />
            </button>
          </div>
        )}
        <div className="center">
          <div className="center-fill" style={{ display: openFile ? "none" : undefined }}>
            <CommitList data={data} wipStats={wipStats} selected={selected} onSelect={select} handleRef={listRef} />
          </div>
          {openFile && (
            <DiffView
              path={path}
              target={openFile}
              headOid={head.oid}
              version={openFile.kind === "wip" ? wipVersion : 0}
              active={active}
              onClose={closeFile}
            />
          )}
        </div>
        <Resizer onDrag={(d) => setDetailW((w) => clamp(w - d, 300, 900))} />
        <div className="detail-col" style={{ width: detailW }}>
          {selected === WIP_ID ? (
            <WipPanel
              path={path}
              wip={wip}
              branch={head.branch}
              viewMode={viewMode}
              onViewMode={setViewMode}
              openFile={openFile}
              onOpenFile={(file, staged) => setOpenFile({ kind: "wip", staged, file })}
              onChanged={refreshWip}
              onCommitted={async (sha) => {
                await load();
                await refreshWip();
                setOpenFile(null);
                setSelected(sha);
                listRef.current?.scrollToId(sha);
              }}
              draft={draft}
              setDraft={setDraft}
            />
          ) : selected ? (
            <CommitPanel
              path={path}
              commitId={selected}
              viewMode={viewMode}
              onViewMode={setViewMode}
              openFilePath={openFile?.kind === "commit" && openFile.commit === selected ? openFile.file.path : null}
              onOpenFile={(file) => setOpenFile({ kind: "commit", commit: selected, file })}
              onSelectCommit={jump}
            />
          ) : (
            <div className="detail">
              <div className="panel-loading">Select a commit</div>
            </div>
          )}
        </div>
      </div>
      {sessions.length > 0 && (
        <TerminalPanel
          cwd={path}
          sessions={sessions}
          activeKey={activeTerm}
          visible={termOpen && active}
          height={termH}
          onResize={(d) => setTermH((h) => clamp(h + d, 120, window.innerHeight - 220))}
          onAdd={addSession}
          onCloseSession={closeSession}
          onActivate={setActiveTerm}
          onHide={() => setTermOpen(false)}
        />
      )}
    </div>
  );
}

interface ToolButtonProps {
  icon: ReactNode;
  label: string;
  title: string;
  onClick: () => void;
  busy?: boolean;
  disabled?: boolean;
  dim?: boolean;
  active?: boolean;
  dot?: boolean;
  badge?: string | null;
}

function ToolButton(p: ToolButtonProps) {
  return (
    <button
      className={`tool-btn${p.dim ? " dim" : ""}${p.active ? " active" : ""}${p.busy ? " busy" : ""}`}
      disabled={p.disabled || p.busy}
      onClick={p.onClick}
      title={p.title}
    >
      <span className="tool-label">{p.label}</span>
      <span className="tool-icon">
        {p.icon}
        {p.dot && <span className="warn-dot" />}
        {p.badge && <span className="tool-badge">{p.badge}</span>}
      </span>
    </button>
  );
}

function PushPrompt({
  remotes,
  initial,
  onCancel,
  onPush,
}: {
  remotes: string[];
  initial: { remote: string; branch: string };
  onCancel: () => void;
  onPush: (remote: string, branch: string) => void;
}) {
  const [remote, setRemote] = useState(initial.remote);
  const [branch, setBranch] = useState(initial.branch);
  return (
    <div className="popover push-prompt" onKeyDown={(e) => e.key === "Escape" && onCancel()}>
      <div className="popover-title">Push and set upstream</div>
      <p className="dim small">“{initial.branch}” isn't tracking a remote branch yet.</p>
      <label className="field">
        <span>Remote</span>
        <select value={remote} onChange={(e) => setRemote(e.target.value)}>
          {remotes.map((r) => (
            <option key={r}>{r}</option>
          ))}
        </select>
      </label>
      <label className="field">
        <span>Remote branch</span>
        <input value={branch} onChange={(e) => setBranch(e.target.value)} autoFocus spellCheck={false} />
      </label>
      <div className="popover-actions">
        <button className="btn" onClick={onCancel}>
          Cancel
        </button>
        <button className="btn primary" disabled={!branch.trim()} onClick={() => onPush(remote, branch.trim())}>
          Push
        </button>
      </div>
    </div>
  );
}
