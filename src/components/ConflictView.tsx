import { useEffect, useMemo, useRef, useState, type ReactNode, type UIEvent } from "react";
import { api, errorText } from "../api";
import { merge3, splitLines, type Conflict } from "../merge3";
import { sideTitle } from "../repoOp";
import type { ConflictEntry, ConflictVersions, RepoOp } from "../types";
import { plural, splitPath } from "../util";
import { showToast } from "../toast";
import { CheckIcon, ChevronIcon, WarningIcon, XIcon } from "./icons";

interface Props {
  path: string;
  entry: ConflictEntry;
  op: RepoOp;
  active: boolean;
  onClose: () => void;
  /** After the file is resolved (refresh WIP and close). */
  onResolved: () => Promise<void>;
}

/** The 3-way editor for a conflicted text file, or a keep/delete choice for anything else. */
export function ConflictView({ path, entry, op, active, onClose, onResolved }: Props) {
  const [versions, setVersions] = useState<ConflictVersions | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!active) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !(e.target instanceof HTMLTextAreaElement || e.target instanceof HTMLInputElement)) onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [active, onClose]);

  useEffect(() => {
    let live = true;
    setVersions(null);
    setError(null);
    api
      .conflictVersions(path, entry.path)
      .then((v) => live && setVersions(v))
      .catch((e) => live && setError(errorText(e)));
    return () => {
      live = false;
    };
  }, [path, entry.path]);

  const resolve = async (fn: () => Promise<void>, done: string) => {
    try {
      await fn();
      showToast(done, "success");
      await onResolved();
    } catch (e) {
      showToast(errorText(e), "error");
    }
  };

  const [dir, base] = splitPath(entry.path);
  const head = (extra?: ReactNode) => (
    <div className="diff-head">
      <WarningIcon size={15} className="st st-conflict" />
      <span className="diff-path" title={entry.path}>
        <span dir="ltr">
          <span className="dim">{dir}</span>
          {base}
        </span>
      </span>
      {extra}
      <button className="icon-btn" onClick={onClose} title="Close (Esc)">
        <XIcon size={16} />
      </button>
    </div>
  );

  if (error || !versions) {
    return (
      <div className="diff-view">
        {head()}
        {error ? <div className="panel-error">{error}</div> : <div className="panel-loading">Loading…</div>}
      </div>
    );
  }

  const { ours, theirs } = versions;
  const text = (entry.kind === "both-modified" || entry.kind === "both-added") && ours && theirs && !ours.binary && !theirs.binary;
  if (!text) {
    return (
      <div className="diff-view">
        {head()}
        <ChoiceBody entry={entry} op={op} versions={versions} onTake={(side, done) => resolve(() => api.resolveTake(path, entry.path, side), done)} />
      </div>
    );
  }
  return (
    <MergeEditor
      key={entry.path}
      versions={versions}
      op={op}
      head={head}
      onSave={(content) => resolve(() => api.resolveFile(path, entry.path, content), `Resolved ${base}`)}
    />
  );
}

// ---------------------------------------------------------------------------
// Text conflicts

/** Picked line indexes from each side of one conflict; -1 = "take this side's (empty) block". */
interface Pick {
  a: number[];
  b: number[];
}

interface OutRow {
  text: string;
  conflict: number | null;
  side: "A" | "B" | null;
  /** Shown but not part of the file (marks an empty conflict). */
  placeholder?: string;
}

const noPicks = (n: number): Pick[] => Array.from({ length: n }, () => ({ a: [], b: [] }));
const picked = (p: Pick) => p.a.length + p.b.length > 0;

function MergeEditor({
  versions,
  op,
  head,
  onSave,
}: {
  versions: ConflictVersions;
  op: RepoOp;
  head: (extra?: ReactNode) => ReactNode;
  onSave: (content: string) => Promise<void>;
}) {
  const oursText = versions.ours!.content;
  const theirsText = versions.theirs!.content;
  const oursLines = useMemo(() => splitLines(oursText), [oursText]);
  const theirsLines = useMemo(() => splitLines(theirsText), [theirsText]);
  const chunks = useMemo(
    () => merge3(splitLines(versions.base?.content ?? ""), oursLines, theirsLines),
    [versions.base?.content, oursLines, theirsLines],
  );
  const conflicts = useMemo(() => chunks.filter((c): c is Conflict => c.kind === "conflict"), [chunks]);
  const eol = (oursText || theirsText).endsWith("\n");

  const [picks, setPicks] = useState<Pick[]>(() => noPicks(conflicts.length));
  const [edited, setEdited] = useState<string | null>(null);
  const [current, setCurrent] = useState(0);
  const [saving, setSaving] = useState(false);
  const [confirm, setConfirm] = useState(false);
  const refs = { a: useRef<HTMLDivElement>(null), b: useRef<HTMLDivElement>(null), out: useRef<HTMLDivElement>(null) };
  const syncing = useRef(false);

  const rows = useMemo(() => {
    const out: OutRow[] = [];
    let ci = 0;
    for (const chunk of chunks) {
      if (chunk.kind === "same") {
        for (const t of chunk.lines) out.push({ text: t, conflict: null, side: null });
        continue;
      }
      const i = ci++;
      const p = picks[i];
      if (!picked(p)) {
        for (const t of chunk.base) out.push({ text: t, conflict: i, side: null });
        if (chunk.base.length === 0) out.push({ text: "", conflict: i, side: null, placeholder: "unresolved conflict" });
        continue;
      }
      const before = out.length;
      for (const j of [...p.a].sort((x, y) => x - y)) if (j >= 0) out.push({ text: chunk.ours[j], conflict: i, side: "A" });
      for (const j of [...p.b].sort((x, y) => x - y)) if (j >= 0) out.push({ text: chunk.theirs[j], conflict: i, side: "B" });
      if (out.length === before) out.push({ text: "", conflict: i, side: null, placeholder: "lines removed" });
    }
    return out;
  }, [chunks, picks]);

  const composed = useMemo(() => {
    const lines = rows.filter((r) => !r.placeholder).map((r) => r.text);
    return lines.join("\n") + (eol && lines.length > 0 ? "\n" : "");
  }, [rows, eol]);

  const unresolved = edited === null ? picks.filter((p) => !picked(p)).length : 0;
  const locked = edited !== null;

  const goTo = (i: number) => {
    setCurrent(i);
    for (const r of [refs.a, refs.b, refs.out]) {
      r.current?.querySelector(`[data-c="${i}"]`)?.scrollIntoView({ block: "center" });
    }
  };
  useEffect(() => {
    if (conflicts.length > 0) goTo(0);
    // Only on first render: later scrolling is the user's.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const toggleLine = (i: number, side: "a" | "b", j: number) => {
    if (locked) return;
    setCurrent(i);
    setPicks((ps) =>
      ps.map((p, k) => (k !== i ? p : { ...p, [side]: p[side].includes(j) ? p[side].filter((x) => x !== j) : [...p[side], j] })),
    );
  };
  const sideLines = (c: Conflict, side: "a" | "b") => {
    const n = (side === "a" ? c.ours : c.theirs).length;
    return n === 0 ? [-1] : Array.from({ length: n }, (_, j) => j);
  };
  const allOf = (side: "a" | "b") => picks.length > 0 && picks.every((p, i) => p[side].length === sideLines(conflicts[i], side).length);
  const toggleAll = (side: "a" | "b") => {
    if (locked) return;
    const on = !allOf(side);
    setPicks((ps) => ps.map((p, i) => ({ ...p, [side]: on ? sideLines(conflicts[i], side) : [] })));
  };

  const reset = () => {
    setPicks(noPicks(conflicts.length));
    setEdited(null);
  };

  const save = async () => {
    setConfirm(false);
    setSaving(true);
    try {
      await onSave(edited ?? composed);
    } finally {
      setSaving(false);
    }
  };

  const onPaneScroll = (from: "a" | "b") => (e: UIEvent<HTMLDivElement>) => {
    if (syncing.current) {
      syncing.current = false;
      return;
    }
    const other = refs[from === "a" ? "b" : "a"].current;
    if (!other) return;
    syncing.current = true;
    other.scrollTop = e.currentTarget.scrollTop;
    other.scrollLeft = e.currentTarget.scrollLeft;
  };

  const pane = (side: "a" | "b") => {
    const lines = side === "a" ? oursLines : theirsLines;
    const sideOp = side === "a" ? op.ours : op.theirs;
    // Which conflict each line belongs to, and empty conflicts by position.
    const owner = new Map<number, [number, number]>();
    const empty = new Map<number, number[]>();
    conflicts.forEach((c, i) => {
      const start = side === "a" ? c.oursStart : c.theirsStart;
      const block = side === "a" ? c.ours : c.theirs;
      if (block.length === 0) empty.set(start, [...(empty.get(start) ?? []), i]);
      block.forEach((_, j) => owner.set(start + j, [i, j]));
    });
    const out: ReactNode[] = [];
    const emptyRows = (at: number) => {
      for (const i of empty.get(at) ?? []) {
        const on = picks[i][side].includes(-1);
        out.push(
          <div key={`e${at}-${i}`} className={`mline conflict ${side} placeholder${i === current ? " current" : ""}`} data-c={i}>
            <input type="checkbox" checked={on} disabled={locked} onChange={() => toggleLine(i, side, -1)} />
            <span className="ln" />
            <span className="text">(removed on this side)</span>
          </div>,
        );
      }
    };
    lines.forEach((t, n) => {
      emptyRows(n);
      const o = owner.get(n);
      if (!o) {
        out.push(
          <div key={n} className="mline">
            <span className="cb" />
            <span className="ln">{n + 1}</span>
            <span className="text">{t}</span>
          </div>,
        );
        return;
      }
      const [i, j] = o;
      const on = picks[i][side].includes(j);
      out.push(
        <div
          key={n}
          className={`mline conflict ${side}${on ? " on" : ""}${i === current ? " current" : ""}`}
          data-c={j === 0 ? i : undefined}
          onClick={() => toggleLine(i, side, j)}
        >
          <input type="checkbox" checked={on} disabled={locked} readOnly />
          <span className="ln">{n + 1}</span>
          <span className="text">
            {on && <CheckIcon size={11} className="pick-check" />}
            {t}
          </span>
        </div>,
      );
    });
    emptyRows(lines.length);
    return (
      <div className="merge-pane">
        <div className="merge-pane-head">
          <input type="checkbox" checked={allOf(side)} disabled={locked} onChange={() => toggleAll(side)} title="Take this side for every conflict" />
          <span className={`side-badge ${side}`}>{side.toUpperCase()}</span>
          <span className="merge-pane-title">{sideTitle(sideOp)}</span>
        </div>
        <div className="merge-scroll" ref={refs[side]} onScroll={onPaneScroll(side)}>
          <div className="code">{out}</div>
        </div>
      </div>
    );
  };

  let lineNo = 0;
  return (
    <div className="diff-view merge-editor">
      {head(
        <>
          <span className="dim">({plural(conflicts.length, "conflict")})</span>
          <button className="outline-btn green" disabled={saving} onClick={() => (unresolved > 0 ? setConfirm(true) : void save())}>
            Save
          </button>
        </>,
      )}
      <div className="merge-sides">
        {pane("a")}
        {pane("b")}
      </div>
      <div className="merge-out-head">
        <span className="merge-out-title">Output</span>
        {conflicts.length > 0 && (
          <span className="merge-nav">
            <span className="merge-nav-label">
              conflict {current + 1} of {conflicts.length}
            </span>
            <button className="icon-btn boxed" disabled={current === 0} onClick={() => goTo(current - 1)} title="Previous conflict">
              <span className="rot-up">
                <ChevronIcon size={12} />
              </span>
            </button>
            <button
              className="icon-btn boxed"
              disabled={current >= conflicts.length - 1}
              onClick={() => goTo(current + 1)}
              title="Next conflict"
            >
              <ChevronIcon size={12} open />
            </button>
          </span>
        )}
        <span className="merge-out-actions">
          {!locked && (
            <button className="btn small" onClick={() => setEdited(composed)} title="Edit the output by hand">
              Edit
            </button>
          )}
          {(locked || picks.some(picked)) && (
            <button className="btn small" onClick={reset}>
              Reset
            </button>
          )}
        </span>
      </div>
      <div className="merge-scroll merge-out" ref={refs.out}>
        {locked ? (
          <textarea className="merge-edit" value={edited} onChange={(e) => setEdited(e.target.value)} spellCheck={false} autoFocus />
        ) : (
          <div className="code" onDoubleClick={() => setEdited(composed)}>
            {rows.map((r, k) => {
              const first = r.conflict !== null && (k === 0 || rows[k - 1].conflict !== r.conflict);
              const cls =
                r.conflict === null
                  ? ""
                  : `conflict ${r.side ? r.side.toLowerCase() + " on" : "unresolved"}${r.conflict === current ? " current" : ""}`;
              return (
                <div key={k} className={`mline ${cls}${r.placeholder ? " placeholder" : ""}`} data-c={first ? r.conflict! : undefined}>
                  <span className="cb side-mark">{r.side}</span>
                  <span className="ln">{r.placeholder ? "" : ++lineNo}</span>
                  <span className="text">
                    {r.side && <CheckIcon size={11} className="pick-check" />}
                    {r.placeholder ? `(${r.placeholder})` : r.text}
                  </span>
                </div>
              );
            })}
          </div>
        )}
      </div>
      {confirm && (
        <div className="modal-backdrop" onMouseDown={() => setConfirm(false)}>
          <div className="modal discard-modal" onMouseDown={(e) => e.stopPropagation()}>
            <div className="modal-title">Save with unpicked conflicts?</div>
            <p>
              {unresolved === 1 ? "1 conflict has" : `${unresolved} conflicts have`} no lines picked from A or B, so{" "}
              {unresolved === 1 ? "it keeps" : "they keep"} the original (base) version.
            </p>
            <div className="popover-actions">
              <button className="btn" autoFocus onClick={() => setConfirm(false)}>
                Cancel
              </button>
              <button className="btn primary" onClick={() => void save()}>
                Save anyway
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Deleted on one side, binary files, ...

function ChoiceBody({
  entry,
  op,
  versions,
  onTake,
}: {
  entry: ConflictEntry;
  op: RepoOp;
  versions: ConflictVersions;
  onTake: (side: "ours" | "theirs" | "delete", done: string) => Promise<void>;
}) {
  const [busy, setBusy] = useState(false);
  const a = op.ours.label;
  const b = op.theirs.label;
  const name = entry.path.split("/").pop();
  const take = (side: "ours" | "theirs" | "delete", done: string) => {
    setBusy(true);
    void onTake(side, done).finally(() => setBusy(false));
  };

  const keep = (side: "ours" | "theirs") => (
    <button className="btn primary" disabled={busy} onClick={() => take(side, `Kept ${name}`)}>
      Keep {side === "ours" ? "A" : "B"}'s version
    </button>
  );
  const del = (
    <button className="btn danger" disabled={busy} onClick={() => take("delete", `Deleted ${name}`)}>
      Delete file
    </button>
  );
  const [text, buttons, preview] = ((): [string, ReactNode, "ours" | "theirs" | null] => {
    switch (entry.kind) {
      case "deleted-by-us":
        return [`Deleted on A (${a}), changed on B (${b}).`, [keep("theirs"), del], "theirs"];
      case "deleted-by-them":
        return [`Changed on A (${a}), deleted on B (${b}).`, [keep("ours"), del], "ours"];
      case "added-by-us":
        return [`Only A (${a}) has this file here; B moved or deleted it.`, [keep("ours"), del], "ours"];
      case "added-by-them":
        return [`Only B (${b}) has this file here; A moved or deleted it.`, [keep("theirs"), del], "theirs"];
      case "both-deleted":
        return ["Deleted on both sides.", [del], null];
      default:
        return ["This binary file changed on both sides. Pick one version.", [keep("ours"), keep("theirs")], null];
    }
  })();
  const content = preview && versions[preview];

  return (
    <div className="diff-body">
      <div className="conflict-choice">
        <p>{text}</p>
        <div className="conflict-choice-actions">
          {Array.isArray(buttons) ? buttons.map((btn, i) => <span key={i}>{btn}</span>) : buttons}
        </div>
      </div>
      {content && !content.binary && (
        <div className="code">
          {splitLines(content.content).map((t, i) => (
            <div key={i} className="line">
              <span className="ln">{i + 1}</span>
              <span className="text">{t}</span>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
