import { useEffect, useState } from "react";
import { api, errorText } from "../api";
import type { DiffTarget, FileContent, FileDiff } from "../types";
import { splitPath } from "../util";
import { StatusIcon, XIcon } from "./icons";

interface Props {
  path: string;
  target: DiffTarget;
  headOid: string | null;
  version: number;
  active: boolean;
  onClose: () => void;
}

type Mode = "diff" | "file";

export function DiffView({ path, target, headOid, version, active, onClose }: Props) {
  const [mode, setMode] = useState<Mode>("diff");
  const [diff, setDiff] = useState<FileDiff | null>(null);
  const [content, setContent] = useState<FileContent | null>(null);
  const [error, setError] = useState<string | null>(null);
  const file = target.file;
  const key = `${target.kind}:${target.kind === "commit" ? target.commit : target.staged}:${file.path}`;

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
    setError(null);
    const load = async () => {
      if (mode === "diff") {
        const d =
          target.kind === "commit"
            ? await api.fileDiff(path, target.commit, file.path, file.oldPath)
            : await api.wipFileDiff(path, file.path, file.oldPath, target.staged);
        if (live) setDiff(d);
      } else {
        let oid: string | null;
        if (target.kind === "commit") oid = target.commit;
        else oid = file.status === "D" ? headOid : null;
        const c = await api.fileContent(path, oid, file.path);
        if (live) setContent(c);
      }
    };
    load().catch((e) => live && setError(errorText(e)));
    return () => {
      live = false;
    };
  }, [path, key, mode, version, headOid]);

  const [dir, base] = splitPath(file.path);

  return (
    <div className="diff-view">
      <div className="diff-head">
        <StatusIcon status={file.status} />
        <span className="diff-path" title={file.oldPath ? `${file.oldPath} → ${file.path}` : file.path}>
          <span dir="ltr">
            {file.oldPath && <span className="dim">{file.oldPath} → </span>}
            <span className="dim">{dir}</span>
            {base}
          </span>
        </span>
        {target.kind === "wip" && <span className="badge">{target.staged ? "staged" : "unstaged"}</span>}
        <div className="seg-toggle">
          <button className={mode === "file" ? "on" : ""} onClick={() => setMode("file")}>
            File View
          </button>
          <button className={mode === "diff" ? "on" : ""} onClick={() => setMode("diff")}>
            Diff View
          </button>
        </div>
        <button className="icon-btn" onClick={onClose} title="Close (Esc)">
          <XIcon size={16} />
        </button>
      </div>
      <div className="diff-body">
        {error ? (
          <div className="panel-error">{error}</div>
        ) : mode === "diff" ? (
          <DiffBody diff={diff} />
        ) : (
          <ContentBody content={content} />
        )}
      </div>
    </div>
  );
}

function DiffBody({ diff }: { diff: FileDiff | null }) {
  if (!diff) return <div className="panel-loading">Loading…</div>;
  if (diff.binary) return <div className="diff-note">Binary file — no text diff.</div>;
  if (diff.hunks.length === 0) return <div className="diff-note">No changes to show.</div>;
  return (
    <div className="code">
      {diff.hunks.map((h, i) => (
        <div key={i} className="hunk">
          <div className="hunk-head">{h.header}</div>
          {h.lines.map((l, j) => (
            <div key={j} className={`line ${l.o === "+" ? "add" : l.o === "-" ? "del" : l.o === "\\" ? "meta" : ""}`}>
              <span className="ln">{l.a ?? ""}</span>
              <span className="ln">{l.b ?? ""}</span>
              <span className="sign">{l.o === "\\" ? "" : l.o}</span>
              <span className="text">{l.t}</span>
            </div>
          ))}
        </div>
      ))}
      {diff.truncated && <div className="diff-note">Diff truncated — it is too large to show in full.</div>}
    </div>
  );
}

function ContentBody({ content }: { content: FileContent | null }) {
  if (!content) return <div className="panel-loading">Loading…</div>;
  if (content.binary) return <div className="diff-note">Binary file.</div>;
  const lines = content.content.split("\n");
  if (lines.length > 1 && lines[lines.length - 1] === "") lines.pop();
  return (
    <div className="code">
      {lines.map((t, i) => (
        <div key={i} className="line">
          <span className="ln">{i + 1}</span>
          <span className="text">{t}</span>
        </div>
      ))}
      {content.truncated && <div className="diff-note">File truncated — it is larger than 2 MB.</div>}
    </div>
  );
}
