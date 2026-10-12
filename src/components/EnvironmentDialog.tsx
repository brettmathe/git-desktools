import { useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { api, errorText } from "../api";
import type { RepoCheck, ScanHit, SmartEnv, SmartRepo } from "../types";
import { FolderIcon, PlusIcon, XIcon } from "./icons";

interface Props {
  /** null = create a new environment. */
  env: SmartEnv | null;
  openPaths: string[];
  onClose: () => void;
  onSaved: (env: SmartEnv) => void;
}

const emptyEnv = (): SmartEnv => ({
  version: 0,
  id: "",
  name: "",
  settings: { sharedBase: null },
  repos: [],
  activeFeatureId: null,
  features: [],
  journal: null,
});

const newRepo = (check: RepoCheck): SmartRepo => ({ id: "", path: check.root, label: check.name, baseBranch: null });

export function EnvironmentDialog({ env, openPaths, onClose, onSaved }: Props) {
  const [draft, setDraft] = useState<SmartEnv>(() => (env ? structuredClone(env) : emptyEnv()));
  // Resolved base branch per repo path, for the base field's placeholder.
  const [resolved, setResolved] = useState<Record<string, string | null>>({});
  const [scan, setScan] = useState<{ folder: string; hits: ScanHit[]; picked: Set<string> } | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notes, setNotes] = useState<string[]>([]);

  const envId = env?.id || null;

  useEffect(() => {
    for (const r of draft.repos) {
      api
        .sfValidateRepo(r.path, envId)
        .then((c) => setResolved((m) => ({ ...m, [r.path]: c.baseBranch })))
        .catch(() => setResolved((m) => ({ ...m, [r.path]: null })));
    }
    // Only for the repos the dialog opened with; added repos record theirs when checked.
  }, []);

  const has = (root: string) => draft.repos.some((r) => r.path === root);

  /** Validate and add repos; collect a note for each one that can't be added. */
  const addPaths = async (paths: string[]) => {
    setBusy(true);
    const added: SmartRepo[] = [];
    const problems: string[] = [];
    for (const p of paths) {
      try {
        const c = await api.sfValidateRepo(p, envId);
        if (has(c.root) || added.some((r) => r.path === c.root)) continue;
        if (c.inEnv) {
          problems.push(`${c.name} is already in "${c.inEnv}".`);
          continue;
        }
        setResolved((m) => ({ ...m, [c.root]: c.baseBranch }));
        added.push(newRepo(c));
      } catch (e) {
        problems.push(errorText(e));
      }
    }
    setDraft((d) => ({ ...d, repos: [...d.repos, ...added] }));
    setNotes(problems);
    setBusy(false);
  };

  const addRepo = async () => {
    const picked = await open({ directory: true, multiple: true, title: "Add repositories" });
    const paths = Array.isArray(picked) ? picked : typeof picked === "string" ? [picked] : [];
    if (paths.length) await addPaths(paths);
  };

  const addFromFolder = async () => {
    const picked = await open({ directory: true, title: "Find repositories in a folder" });
    if (typeof picked !== "string") return;
    setBusy(true);
    try {
      const hits = await api.sfScanFolder(picked, envId);
      const picks = new Set(hits.filter((h) => !h.inEnv && !has(h.root)).map((h) => h.root));
      setScan({ folder: picked, hits, picked: picks });
    } catch (e) {
      setNotes([errorText(e)]);
    } finally {
      setBusy(false);
    }
  };

  const update = (i: number, patch: Partial<SmartRepo>) =>
    setDraft((d) => ({ ...d, repos: d.repos.map((r, j) => (j === i ? { ...r, ...patch } : r)) }));

  const save = async () => {
    setBusy(true);
    setError(null);
    try {
      onSaved(await api.sfSaveEnv(draft));
    } catch (e) {
      setError(errorText(e));
      setBusy(false);
    }
  };

  const openNotListed = openPaths.filter((p) => !has(p));

  if (scan) {
    const toggle = (root: string) =>
      setScan((s) => {
        if (!s) return s;
        const picked = new Set(s.picked);
        if (picked.has(root)) picked.delete(root);
        else picked.add(root);
        return { ...s, picked };
      });
    return (
      <div className="modal-backdrop" onMouseDown={() => setScan(null)}>
        <div className="modal" onMouseDown={(e) => e.stopPropagation()}>
          <div className="modal-title">Repositories in {scan.folder.split(/[\\/]/).pop()}</div>
          <div className="dim small mono sf-scan-folder">{scan.folder}</div>
          <div className="sf-scan-list">
            {scan.hits.length === 0 && <div className="lp-empty">No git repositories found up to 3 folders down.</div>}
            {scan.hits.map((h) => {
              const listed = has(h.root);
              const blocked = !!h.inEnv || listed;
              return (
                <label key={h.root} className={`sf-scan-row${blocked ? " blocked" : ""}`} title={h.root}>
                  <input
                    type="checkbox"
                    checked={!blocked && scan.picked.has(h.root)}
                    disabled={blocked}
                    onChange={() => toggle(h.root)}
                  />
                  <span className="sf-scan-name">{h.name}</span>
                  <span className="dim small sf-scan-path">
                    {h.root.slice(scan.folder.length).replace(/^[\\/]/, "")}
                  </span>
                  {h.inEnv && <span className="chip">in {h.inEnv}</span>}
                  {listed && <span className="chip">already added</span>}
                </label>
              );
            })}
          </div>
          <div className="popover-actions">
            <button className="btn" onClick={() => setScan(null)}>
              Cancel
            </button>
            <button
              className="btn primary"
              disabled={scan.picked.size === 0}
              onClick={() => {
                const paths = [...scan.picked];
                setScan(null);
                void addPaths(paths);
              }}
            >
              Add {scan.picked.size} {scan.picked.size === 1 ? "repository" : "repositories"}
            </button>
          </div>
        </div>
      </div>
    );
  }

  return (
    <div className="modal-backdrop" onMouseDown={() => !busy && onClose()}>
      <div className="modal sf-env-modal" onMouseDown={(e) => e.stopPropagation()}>
        <div className="modal-title">{env ? `Edit ${env.name}` : "New Smart Environment"}</div>
        <label className="field">
          <span>Name</span>
          <input
            autoFocus
            value={draft.name}
            placeholder="e.g. RGT"
            spellCheck={false}
            onChange={(e) => setDraft((d) => ({ ...d, name: e.target.value }))}
          />
        </label>

        <div className="sf-repos-head">
          <span>
            Repositories <span className="count">{draft.repos.length}</span>
          </span>
          <span className="sf-add-buttons">
            <button className="btn small" onClick={() => void addRepo()} disabled={busy}>
              <PlusIcon size={12} /> Add repo…
            </button>
            <button className="btn small" onClick={() => void addFromFolder()} disabled={busy}>
              <FolderIcon size={12} /> Add from folder…
            </button>
            <button
              className="btn small"
              onClick={() => void addPaths(openNotListed)}
              disabled={busy || openNotListed.length === 0}
              title="Add the repositories open in tabs"
            >
              Add open tabs
            </button>
          </span>
        </div>
        <div className="sf-repo-list">
          {draft.repos.length === 0 && (
            <div className="lp-empty">
              Add the repositories that make up this environment. They can live anywhere on disk.
            </div>
          )}
          {draft.repos.map((r, i) => (
            <div key={r.id || r.path} className="sf-repo-edit">
              <input
                className="sf-repo-label"
                value={r.label}
                spellCheck={false}
                title="Display name"
                onChange={(e) => update(i, { label: e.target.value })}
              />
              <span className="dim small sf-repo-path" title={r.path}>
                <span dir="ltr">{r.path}</span>
              </span>
              <input
                className="sf-repo-base mono"
                value={r.baseBranch ?? ""}
                spellCheck={false}
                placeholder={
                  resolved[r.path] ? `base: ${resolved[r.path]}` : r.path in resolved ? "base: (none)" : "base: …"
                }
                title="Base branch. Leave empty to use the remote's default branch."
                onChange={(e) => update(i, { baseBranch: e.target.value || null })}
              />
              <button
                className="icon-btn"
                title="Remove from environment"
                onClick={() => setDraft((d) => ({ ...d, repos: d.repos.filter((_, j) => j !== i) }))}
              >
                <XIcon size={13} />
              </button>
            </div>
          ))}
        </div>
        {notes.length > 0 && (
          <div className="sf-notes">
            {notes.map((n) => (
              <div key={n}>{n}</div>
            ))}
          </div>
        )}

        <label className="field sf-shared-base">
          <span>Shared base branch (optional)</span>
          <input
            className="mono"
            value={draft.settings.sharedBase ?? ""}
            placeholder="e.g. release/2.4. Applies to every repo that has it"
            spellCheck={false}
            onChange={(e) =>
              setDraft((d) => ({ ...d, settings: { ...d.settings, sharedBase: e.target.value || null } }))
            }
          />
        </label>

        {error && <pre className="commit-error">{error}</pre>}
        <div className="popover-actions">
          <button className="btn" onClick={onClose} disabled={busy}>
            Cancel
          </button>
          <button className="btn primary" onClick={() => void save()} disabled={busy || !draft.name.trim()}>
            {env ? "Save" : "Create environment"}
          </button>
        </div>
      </div>
    </div>
  );
}
