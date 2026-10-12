import { useState, type KeyboardEvent } from "react";
import { api, errorText } from "../api";
import type { CommitGuard, FileChange, FileViewMode, OpenFile, WipStatus } from "../types";
import { plural } from "../util";
import { showToast } from "../toast";
import { FileList, ViewModeToggle } from "./FileList";
import { ChevronIcon } from "./icons";

export interface CommitDraft {
  summary: string;
  description: string;
  amend: boolean;
  saved: { summary: string; description: string } | null;
}

export const emptyDraft: CommitDraft = { summary: "", description: "", amend: false, saved: null };
const SUMMARY_LIMIT = 72;

interface Props {
  path: string;
  wip: WipStatus;
  branch: string | null;
  viewMode: FileViewMode;
  onViewMode: (m: FileViewMode) => void;
  openFile: OpenFile | null;
  onOpenFile: (file: FileChange, staged: boolean) => void;
  onChanged: () => Promise<void>;
  onCommitted: (sha: string) => Promise<void>;
  draft: CommitDraft;
  setDraft: (fn: (d: CommitDraft) => CommitDraft) => void;
}

export function WipPanel(p: Props) {
  const { wip, draft, setDraft } = p;
  const [busy, setBusy] = useState(false);
  const [commitError, setCommitError] = useState<string | null>(null);
  // Smart Features: this commit would land on base while a feature is active.
  const [guard, setGuard] = useState<CommitGuard | null>(null);
  const [open, setOpen] = useState({ unstaged: true, staged: true });
  const total = new Set([...wip.unstaged, ...wip.staged].map((f) => f.path)).size;

  const run = async (fn: () => Promise<void>) => {
    setBusy(true);
    try {
      await fn();
    } catch (e) {
      showToast(errorText(e), "error");
    } finally {
      await p.onChanged();
      setBusy(false);
    }
  };
  const stage = (files: string[]) => run(() => api.stageFiles(p.path, files));
  const unstage = (files: string[]) => run(() => api.unstageFiles(p.path, files));

  const toggleAmend = async (amend: boolean) => {
    if (amend) {
      try {
        const msg = await api.headMessage(p.path);
        const [summary, ...rest] = msg.split("\n");
        setDraft((d) => ({
          amend: true,
          saved: { summary: d.summary, description: d.description },
          summary: summary.trim(),
          description: rest.join("\n").trim(),
        }));
      } catch (e) {
        showToast(errorText(e), "error");
      }
    } else {
      setDraft((d) => ({ amend: false, saved: null, summary: d.saved?.summary ?? "", description: d.saved?.description ?? "" }));
    }
  };

  const staged = wip.staged.length;
  const hasSummary = draft.summary.trim().length > 0;
  const canCommit = !busy && hasSummary && (staged > 0 || draft.amend);
  const label = busy
    ? "Working…"
    : !draft.amend && staged === 0
      ? "Stage Changes to Commit"
      : !hasSummary
        ? "Type a Message to Commit"
        : draft.amend
          ? "Amend Previous Commit"
          : `Commit Changes to ${plural(staged, "File")}`;

  const doCommit = async (moveTo?: CommitGuard) => {
    if (!canCommit) return;
    setBusy(true);
    setCommitError(null);
    setGuard(null);
    const message = draft.description.trim()
      ? `${draft.summary.trim()}\n\n${draft.description.trim()}\n`
      : `${draft.summary.trim()}\n`;
    try {
      if (moveTo) {
        await api.sfMoveToFeature(moveTo.envId, moveTo.repoId);
      } else {
        const g = await api.sfCommitGuard(p.path).catch(() => null);
        if (g) {
          setGuard(g);
          return;
        }
      }
      const sha = await api.commit(p.path, message, draft.amend);
      setDraft(() => emptyDraft);
      showToast(draft.amend ? "Amended the previous commit" : `Committed ${plural(staged, "file")}`, "success");
      await p.onCommitted(sha);
    } catch (e) {
      setCommitError(errorText(e));
    } finally {
      setBusy(false);
    }
  };

  const onKey = (e: KeyboardEvent) => {
    if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
      e.preventDefault();
      void doCommit();
    }
  };

  const remaining = SUMMARY_LIMIT - draft.summary.length;
  const openPath = (staged: boolean) =>
    p.openFile?.kind === "wip" && p.openFile.staged === staged ? p.openFile.file.path : null;

  return (
    <div className="detail wip-panel">
      <div className="detail-head centered">
        <span>
          {plural(total, "file change")} on{" "}
          {p.branch ? <span className="branch-chip">{p.branch}</span> : <span className="dim">detached HEAD</span>}
        </span>
      </div>
      <div className="files-bar end">
        <ViewModeToggle mode={p.viewMode} onChange={p.onViewMode} />
      </div>
      <div className="wip-sections">
        <section className={`wip-section${open.unstaged ? " open" : ""}`}>
          <div className="wip-section-head">
            <button className="plain" onClick={() => setOpen((o) => ({ ...o, unstaged: !o.unstaged }))}>
              <ChevronIcon size={12} open={open.unstaged} /> Unstaged Files ({wip.unstaged.length})
            </button>
            {wip.unstaged.length > 0 && (
              <button className="outline-btn green" disabled={busy} onClick={() => stage([])}>
                Stage All Changes
              </button>
            )}
          </div>
          {open.unstaged && (
            <div className="wip-files">
              <FileList
                files={wip.unstaged}
                mode={p.viewMode}
                selectedPath={openPath(false)}
                onOpen={(f) => p.onOpenFile(f, false)}
                action={(f) => (
                  <button
                    className="mini-btn green"
                    disabled={busy}
                    onClick={(e) => {
                      e.stopPropagation();
                      void stage([f.path]);
                    }}
                  >
                    Stage
                  </button>
                )}
              />
            </div>
          )}
        </section>
        <section className={`wip-section${open.staged ? " open" : ""}`}>
          <div className="wip-section-head">
            <button className="plain" onClick={() => setOpen((o) => ({ ...o, staged: !o.staged }))}>
              <ChevronIcon size={12} open={open.staged} /> Staged Files ({wip.staged.length})
            </button>
            {wip.staged.length > 0 && (
              <button className="outline-btn red" disabled={busy} onClick={() => unstage([])}>
                Unstage All Changes
              </button>
            )}
          </div>
          {open.staged && (
            <div className="wip-files">
              <FileList
                files={wip.staged}
                mode={p.viewMode}
                selectedPath={openPath(true)}
                onOpen={(f) => p.onOpenFile(f, true)}
                action={(f) => (
                  <button
                    className="mini-btn red"
                    disabled={busy}
                    onClick={(e) => {
                      e.stopPropagation();
                      void unstage(f.oldPath ? [f.path, f.oldPath] : [f.path]);
                    }}
                  >
                    Unstage
                  </button>
                )}
              />
            </div>
          )}
        </section>
      </div>
      <div className="commit-box" onKeyDown={onKey}>
        <label className="amend">
          <input type="checkbox" checked={draft.amend} disabled={busy} onChange={(e) => void toggleAmend(e.target.checked)} />
          Amend previous commit
        </label>
        <div className="commit-inputs">
          <div className="summary-row">
            <input
              className="summary-input"
              placeholder="Commit summary"
              value={draft.summary}
              onChange={(e) => setDraft((d) => ({ ...d, summary: e.target.value }))}
              spellCheck
            />
            <span className={`counter${remaining < 0 ? " over" : remaining < 10 ? " near" : ""}`}>{remaining}</span>
          </div>
          <textarea
            className="desc-input"
            placeholder="Description"
            value={draft.description}
            onChange={(e) => setDraft((d) => ({ ...d, description: e.target.value }))}
          />
        </div>
        {commitError && <pre className="commit-error">{commitError}</pre>}
        {guard && (
          <div className="commit-guard">
            <div>
              <b>{guard.featureName}</b> is active in {guard.envName}, so this commit belongs on{" "}
              <span className="mono">{guard.branch}</span>, not <span className="mono">{guard.base}</span>.
            </div>
            <div className="commit-guard-actions">
              <button className="btn small" onClick={() => setGuard(null)}>
                Cancel
              </button>
              <button className="btn small primary" onClick={() => void doCommit(guard)}>
                Move to {guard.branch} and commit
              </button>
            </div>
          </div>
        )}
        <button className="commit-btn" disabled={!canCommit} onClick={() => void doCommit()} title="⌘/Ctrl + Enter">
          {label}
        </button>
      </div>
    </div>
  );
}
