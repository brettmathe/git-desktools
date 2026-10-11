import { useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { api, errorText } from "../api";
import type { RecentRepo, RepoInfo, RepoSummary } from "../types";
import { showToast } from "../toast";
import { CloneDialog } from "./CloneDialog";
import { BranchIcon, CloudIcon, FolderIcon, PlusIcon, SearchIcon, XIcon } from "./icons";

interface Props {
  recent: RecentRepo[];
  openPaths: string[];
  cloneDir: string;
  onOpen: (path: string) => void;
  onRemoveRecent: (path: string) => void;
  onCloned: (info: RepoInfo, parentDir: string) => void;
}

const summaries = new Map<string, RepoSummary | "missing">();

function useSummaries(paths: string[]) {
  const [, rerender] = useState(0);
  useEffect(() => {
    let live = true;
    for (const p of paths) {
      if (summaries.has(p)) continue;
      api
        .repoSummary(p)
        .then((s) => summaries.set(p, s))
        .catch(() => summaries.set(p, "missing"))
        .finally(() => live && rerender((n) => n + 1));
    }
    return () => {
      live = false;
    };
  }, [paths.join("\n")]);
  return summaries;
}

export function Launchpad({ recent, openPaths, cloneDir, onOpen, onRemoveRecent, onCloned }: Props) {
  const [query, setQuery] = useState("");
  const [cloning, setCloning] = useState(false);
  const all = [...new Set([...openPaths, ...recent.map((r) => r.path)])];
  const info = useSummaries(all);

  const q = query.trim().toLowerCase();
  const matches = (path: string) => {
    if (!q) return true;
    const s = info.get(path);
    const owner = s && s !== "missing" ? (s.owner ?? "") : "";
    return path.toLowerCase().includes(q) || owner.toLowerCase().includes(q);
  };

  const browse = async () => {
    const picked = await open({ directory: true, title: "Open a git repository" });
    if (typeof picked === "string") onOpen(picked);
  };

  const init = async () => {
    const picked = await open({ directory: true, title: "Choose a folder to initialize" });
    if (typeof picked !== "string") return;
    try {
      const repo = await api.initRepo(picked);
      summaries.delete(repo.root);
      onOpen(repo.root);
    } catch (e) {
      showToast(errorText(e), "error");
    }
  };

  const row = (path: string, removable: boolean) => {
    const s = info.get(path);
    const name = path.split(/[\\/]/).pop() ?? path;
    const missing = s === "missing";
    return (
      <div key={path} className={`lp-row${missing ? " missing" : ""}`} onDoubleClick={() => !missing && onOpen(path)}>
        <button className="lp-name link-btn" onClick={() => onOpen(path)} disabled={missing} title={path}>
          {name}
        </button>
        <span className="lp-owner">{s && s !== "missing" ? s.owner : ""}</span>
        <span className="lp-path dim" title={path}>
          <span dir="ltr">{missing ? "Not found" : path}</span>
        </span>
        <span className="lp-chips">
          {s && s !== "missing" && s.branch && (
            <span className="chip">
              <BranchIcon size={11} /> {s.branch}
            </span>
          )}
          {s && s !== "missing" && s.ahead > 0 && <span className="chip ahead">↑{s.ahead}</span>}
          {s && s !== "missing" && s.behind > 0 && <span className="chip behind">↓{s.behind}</span>}
        </span>
        <span className="lp-actions">
          <button className="btn small" onClick={() => onOpen(path)} disabled={missing}>
            Open
          </button>
          {removable && (
            <button className="icon-btn" title="Remove from recent" onClick={() => onRemoveRecent(path)}>
              <XIcon size={13} />
            </button>
          )}
        </span>
      </div>
    );
  };

  const openList = openPaths.filter(matches);
  const recentList = recent.map((r) => r.path).filter(matches);

  return (
    <div className="launchpad">
      <h1>Repository Management</h1>
      <div className="lp-buttons">
        <button className="btn primary" onClick={() => void browse()}>
          <FolderIcon size={14} /> Browse
        </button>
        <button className="btn primary" onClick={() => setCloning(true)}>
          <CloudIcon size={14} /> Clone
        </button>
        <button className="btn primary" onClick={() => void init()}>
          <PlusIcon size={14} /> Init
        </button>
      </div>
      <div className="lp-search">
        <SearchIcon size={14} />
        <input value={query} onChange={(e) => setQuery(e.target.value)} placeholder="Search repositories" spellCheck={false} />
      </div>
      <section className="lp-section">
        <div className="lp-head">
          Open repositories <span className="count">{openPaths.length}</span>
        </div>
        {openList.length ? openList.map((p) => row(p, false)) : <div className="lp-empty">{q ? "No matches" : "No repositories open"}</div>}
      </section>
      <section className="lp-section">
        <div className="lp-head">
          Recent repositories <span className="count">{recent.length}</span>
        </div>
        {recentList.length ? (
          recentList.map((p) => row(p, true))
        ) : (
          <div className="lp-empty">{q ? "No matches" : "Repositories you open will show up here."}</div>
        )}
      </section>
      {cloning && (
        <CloneDialog
          defaultDir={cloneDir}
          onClose={() => setCloning(false)}
          onCloned={(repo, dir) => {
            setCloning(false);
            onCloned(repo, dir);
          }}
        />
      )}
    </div>
  );
}
