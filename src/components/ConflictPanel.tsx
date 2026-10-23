import { useEffect, useRef, useState, type ReactNode } from "react";
import { api, errorText } from "../api";
import { abortDetail, abortLabel, continueLabel, opKey, opTarget, opTitle } from "../repoOp";
import type { ConflictEntry, FileChange, FileViewMode, OpenFile, RepoOp, WipStatus } from "../types";
import { plural } from "../util";
import { showToast } from "../toast";
import { CommitBox, draftMessage, messageDraft, type CommitDraft } from "./CommitBox";
import { FileList, ViewModeToggle, type MenuItem } from "./FileList";
import { ChevronIcon, WarningIcon } from "./icons";

interface Props {
  path: string;
  wip: WipStatus;
  op: RepoOp;
  viewMode: FileViewMode;
  onViewMode: (m: FileViewMode) => void;
  openFile: OpenFile | null;
  onOpenConflict: (entry: ConflictEntry) => void;
  onOpenFile: (file: FileChange, staged: boolean) => void;
  onChanged: () => Promise<void>;
  /** After Continue, Skip or Abort: `sha` is the new HEAD when a commit was made. */
  onFinished: (sha: string | null) => Promise<void>;
}

const asChange = (c: ConflictEntry): FileChange => ({
  path: c.path,
  oldPath: null,
  status: "U",
  additions: 0,
  deletions: 0,
  binary: false,
});

type Confirm = { title: string; body: ReactNode; action: string; danger?: boolean; run: () => Promise<void> };

/** Right panel while a merge, rebase, cherry-pick, ... is in progress. */
export function ConflictPanel(p: Props) {
  const { op, wip } = p;
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [confirm, setConfirm] = useState<Confirm | null>(null);
  const [open, setOpen] = useState({ conflicted: true, resolved: true, unstaged: true });
  const key = opKey(op);
  const [draft, setDraftState] = useState<{ key: string; draft: CommitDraft }>(() => ({ key, draft: messageDraft(op.message) }));
  // Files that were conflicted during this stop, so "Mark conflicted" is only offered for them.
  const wasConflicted = useRef<{ key: string; paths: Set<string> }>({ key, paths: new Set() });

  // The operation moved on (next rebase step, another merge): start from git's new message.
  useEffect(() => {
    if (draft.key !== key) setDraftState({ key, draft: messageDraft(op.message) });
    setError(null);
  }, [key]); // eslint-disable-line react-hooks/exhaustive-deps
  if (wasConflicted.current.key !== key) wasConflicted.current = { key, paths: new Set() };
  for (const c of op.conflicts) wasConflicted.current.paths.add(c.path);

  const setDraft = (fn: (d: CommitDraft) => CommitDraft) => setDraftState((s) => ({ ...s, draft: fn(s.draft) }));

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

  const markResolved = (files: string[]) => run(() => api.markResolved(p.path, files));
  const markResolvedChecked = async (files: string[]) => {
    const marked = await api.conflictMarkers(p.path, files).catch(() => []);
    if (marked.length === 0) return markResolved(files);
    setConfirm({
      title: marked.length === 1 ? "This file still has conflict markers" : `${marked.length} files still have conflict markers`,
      body: (
        <>
          <div className="dim small mono discard-path">{marked.join("\n")}</div>
          <p>They still contain &lt;&lt;&lt;&lt;&lt;&lt;&lt; … &gt;&gt;&gt;&gt;&gt;&gt;&gt; blocks. Mark them resolved anyway?</p>
        </>
      ),
      action: "Mark Resolved",
      run: () => markResolved(files),
    });
  };
  const take = (file: string, side: "ours" | "theirs") => run(() => api.resolveTake(p.path, file, side));

  const conflictMenu = (t: { file: FileChange } | { folder: string; files: FileChange[] }): MenuItem[] => {
    if (!("file" in t)) return [{ label: "Mark folder resolved", onClick: () => void markResolvedChecked(t.files.map((f) => f.path)), disabled: busy }];
    const f = t.file.path;
    const kind = op.conflicts.find((c) => c.path === f)?.kind;
    return [
      { label: "Mark resolved", onClick: () => void markResolvedChecked([f]), disabled: busy },
      { label: "Take A (ours)", onClick: () => void take(f, "ours"), disabled: busy || kind === "deleted-by-us" || kind === "added-by-them" },
      { label: "Take B (theirs)", onClick: () => void take(f, "theirs"), disabled: busy || kind === "deleted-by-them" || kind === "added-by-us" },
      { label: "Copy file path", onClick: () => void navigator.clipboard.writeText(f).then(() => showToast("Copied file path", "success")) },
    ];
  };

  const finish = async (fn: () => Promise<string | null>, done: (sha: string | null) => string) => {
    setBusy(true);
    setError(null);
    try {
      const sha = await fn();
      showToast(done(sha), "success");
      await p.onFinished(sha);
    } catch (e) {
      setError(errorText(e));
      await p.onChanged();
    } finally {
      setBusy(false);
    }
  };

  const conflicts = op.conflicts.length;
  const hasSummary = draft.draft.summary.trim().length > 0;
  const canContinue = !busy && conflicts === 0 && (hasSummary || !op.messageEditable);
  const doContinue = () => {
    if (!canContinue) return;
    void finish(
      () => api.continueOperation(p.path, op.messageEditable ? draftMessage(draft.draft) : ""),
      () => (op.kind === "merge" ? `Merged ${op.theirs.label} into ${opTarget(op)}` : `Continued the ${op.kind}`),
    );
  };
  const doSkip = () =>
    setConfirm({
      title: "Skip this commit?",
      body: <p>{op.theirs.label} is left out, and its changes (including your resolutions) are dropped.</p>,
      action: "Skip",
      danger: true,
      run: () => finish(() => api.skipOperation(p.path), () => "Skipped the commit"),
    });
  const doAbort = () =>
    setConfirm({
      title: `${abortLabel(op)}?`,
      body: <p>{abortDetail(op)}</p>,
      action: abortLabel(op),
      danger: true,
      run: () =>
        finish(
          async () => {
            await api.abortOperation(p.path);
            return null;
          },
          () => (op.kind === "stash" ? "Undid the stash apply" : `Aborted the ${op.kind}`),
        ),
    });

  // A file resolved to HEAD's version has nothing staged; still list it as resolved.
  const unchanged = [...wasConflicted.current.paths]
    .filter((f) => !op.conflicts.some((c) => c.path === f) && !wip.staged.some((s) => s.path === f))
    .sort();
  const unchangedKey = unchanged.join("\n");
  // Their real status: deleted (red ✖) when the resolution removed the file.
  const [unchangedFiles, setUnchangedFiles] = useState<FileChange[]>([]);
  useEffect(() => {
    let live = true;
    if (unchanged.length === 0) setUnchangedFiles([]);
    else
      api
        .resolvedStatus(p.path, unchanged)
        .then((fs) => live && setUnchangedFiles(fs))
        .catch(() => live && setUnchangedFiles(unchanged.map((f) => ({ ...asChange({ path: f, kind: "both-modified" }), status: "M" }))));
    return () => {
      live = false;
    };
  }, [p.path, unchangedKey]); // eslint-disable-line react-hooks/exhaustive-deps
  const resolved = [...wip.staged, ...unchangedFiles.filter((f) => unchanged.includes(f.path))].sort((a, b) =>
    a.path.localeCompare(b.path),
  );

  const openConflict = p.openFile?.kind === "conflict" ? p.openFile.file.path : null;
  const openStaged = p.openFile?.kind === "wip" && p.openFile.staged ? p.openFile.file.path : null;
  const openUnstaged = p.openFile?.kind === "wip" && !p.openFile.staged ? p.openFile.file.path : null;

  const section = (
    id: keyof typeof open,
    title: string,
    count: number,
    button: ReactNode,
    list: ReactNode,
  ) => (
    <section className={`wip-section${open[id] ? " open" : ""}`}>
      <div className="wip-section-head">
        <button className="plain" onClick={() => setOpen((o) => ({ ...o, [id]: !o[id] }))}>
          <ChevronIcon size={12} open={open[id]} /> {title} ({count})
        </button>
        {count > 0 && button}
      </div>
      {open[id] && <div className="wip-files">{list}</div>}
    </section>
  );

  return (
    <div className="detail wip-panel conflict-panel">
      <div className="detail-head centered conflict-title">
        <WarningIcon size={15} className="st-amber" /> {opTitle(op)}
      </div>
      <div className="op-desc">
        <OpDescription op={op} />
      </div>
      <div className="files-bar end">
        <ViewModeToggle mode={p.viewMode} onChange={p.onViewMode} />
      </div>
      <div className="wip-sections">
        {section(
          "conflicted",
          "Conflicted Files",
          conflicts,
          <button className="outline-btn amber" disabled={busy} onClick={() => void markResolvedChecked([])}>
            Mark All Resolved
          </button>,
          conflicts === 0 ? (
            <div className="empty-note small">All conflicts are resolved.</div>
          ) : (
            <FileList
              files={op.conflicts.map(asChange)}
              mode={p.viewMode}
              selectedPath={openConflict}
              onOpen={(f) => p.onOpenConflict(op.conflicts.find((c) => c.path === f.path)!)}
              contextMenu={conflictMenu}
            />
          ),
        )}
        {section(
          "resolved",
          "Resolved Files",
          resolved.length,
          null,
          <FileList
            files={resolved}
            mode={p.viewMode}
            selectedPath={openStaged}
            onOpen={(f) => p.onOpenFile(f, true)}
            action={(f) =>
              wasConflicted.current.paths.has(f.path) && (
                <button
                  className="mini-btn amber"
                  disabled={busy}
                  onClick={(e) => {
                    e.stopPropagation();
                    void run(() => api.markConflicted(p.path, f.path));
                  }}
                >
                  Mark conflicted
                </button>
              )
            }
          />,
        )}
        {wip.unstaged.length > 0 &&
          section(
            "unstaged",
            "Unstaged Files",
            wip.unstaged.length,
            <button className="outline-btn green" disabled={busy} onClick={() => void run(() => api.stageFiles(p.path, wip.unstaged.map((f) => f.path)))}>
              Stage All Changes
            </button>,
            <FileList
              files={wip.unstaged}
              mode={p.viewMode}
              selectedPath={openUnstaged}
              onOpen={(f) => p.onOpenFile(f, false)}
              action={(f) => (
                <button
                  className="mini-btn green"
                  disabled={busy}
                  onClick={(e) => {
                    e.stopPropagation();
                    void run(() => api.stageFiles(p.path, [f.path]));
                  }}
                >
                  Stage
                </button>
              )}
            />,
          )}
      </div>
      {op.kind === "stash" ? (
        <div className="commit-box">
          <p className="dim small op-note">
            Resolve each file to keep the stash's changes. The stash entry is kept; drop it in the terminal with{" "}
            <span className="mono">git stash drop</span> once you're done.
          </p>
          {error && <pre className="commit-error">{error}</pre>}
          <div className="op-buttons">
            <button className="abort-btn" disabled={busy} onClick={doAbort}>
              {abortLabel(op)}
            </button>
          </div>
        </div>
      ) : (
        <CommitBox draft={draft.draft} setDraft={setDraft} busy={busy} readOnly={!op.messageEditable} error={error} onSubmit={doContinue}>
          <div className="op-buttons">
            <button
              className="commit-btn"
              disabled={!canContinue}
              onClick={doContinue}
              title={conflicts > 0 ? `Resolve ${plural(conflicts, "conflicted file")} first` : "⌘/Ctrl + Enter"}
            >
              {busy ? "Working…" : continueLabel(op)}
            </button>
            {op.canSkip && (
              <button className="skip-btn" disabled={busy} onClick={doSkip} title={`Leave ${op.theirs.label} out`}>
                Skip
              </button>
            )}
            <button className="abort-btn" disabled={busy} onClick={doAbort}>
              {abortLabel(op)}
            </button>
          </div>
        </CommitBox>
      )}
      {confirm && (
        <div className="modal-backdrop" onMouseDown={() => setConfirm(null)}>
          <div className="modal discard-modal" onMouseDown={(e) => e.stopPropagation()}>
            <div className="modal-title">{confirm.title}</div>
            {confirm.body}
            <div className="popover-actions">
              <button className="btn" autoFocus onClick={() => setConfirm(null)}>
                Cancel
              </button>
              <button
                className={`btn ${confirm.danger ? "danger" : "primary"}`}
                onClick={() => {
                  const c = confirm;
                  setConfirm(null);
                  void c.run();
                }}
              >
                {confirm.action}
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}

function OpDescription({ op }: { op: RepoOp }) {
  const chip = (text: string, theirs = false) => <span className={`branch-chip${theirs ? " theirs" : ""}`}>{text}</span>;
  const target = chip(opTarget(op));
  const step = op.step && op.total ? <span className="dim"> ({op.step} of {op.total})</span> : null;
  switch (op.kind) {
    case "merge":
      return <>Merging {chip(op.theirs.label, true)} into {target}</>;
    case "cherry-pick":
      return <>Cherry-picking {chip(op.theirs.label, true)} onto {target}</>;
    case "revert":
      return <>Reverting {chip(op.theirs.label, true)} on {target}</>;
    case "rebase":
      return (
        <>
          Rebasing {target} onto {chip(op.ours.label)}
          {step}
          <div className="op-sub">
            Replaying {chip(op.theirs.label, true)}
          </div>
        </>
      );
    case "am":
      return <>Applying {chip(op.theirs.label, true)} to {target}</>;
    case "stash":
      return <>Applying a stash to {target}</>;
  }
}
