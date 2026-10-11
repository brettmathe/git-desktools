import { useEffect, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { api, errorText, onProgress } from "../api";
import type { RepoInfo } from "../types";
import { normalizeCloneUrl } from "../util";

interface Props {
  defaultDir: string;
  onClose: () => void;
  onCloned: (info: RepoInfo, parentDir: string) => void;
}

export function CloneDialog({ defaultDir, onClose, onCloned }: Props) {
  const [url, setUrl] = useState("");
  const [dir, setDir] = useState(defaultDir);
  const [name, setName] = useState("");
  const [nameTouched, setNameTouched] = useState(false);
  const [running, setRunning] = useState(false);
  const [log, setLog] = useState<string[]>([]);
  const [error, setError] = useState<string | null>(null);
  const logRef = useRef<HTMLPreElement>(null);

  useEffect(() => {
    if (!nameTouched) setName(normalizeCloneUrl(url).name);
  }, [url, nameTouched]);

  useEffect(() => {
    logRef.current?.scrollTo(0, logRef.current.scrollHeight);
  }, [log]);

  const sep = dir.includes("\\") ? "\\" : "/";
  const dest = `${dir.replace(/[\\/]+$/, "")}${sep}${name}`;

  const browse = async () => {
    const picked = await open({ directory: true, defaultPath: dir, title: "Clone into…" });
    if (typeof picked === "string") setDir(picked);
  };

  const start = async () => {
    const { url: cloneUrl } = normalizeCloneUrl(url);
    setRunning(true);
    setError(null);
    setLog([`git clone ${cloneUrl}`]);
    const unlisten = await onProgress("clone-progress", dest, (line) =>
      setLog((l) => {
        // git rewrites progress lines in place with \r; keep only the latest of each phase.
        const phase = line.split(":")[0];
        const prev = l[l.length - 1];
        return prev && prev.split(":")[0] === phase ? [...l.slice(0, -1), line] : [...l, line];
      }),
    );
    try {
      const info = await api.cloneRepo(cloneUrl, dir, name);
      onCloned(info, dir);
    } catch (e) {
      setError(errorText(e));
    } finally {
      unlisten();
      setRunning(false);
    }
  };

  const valid = url.trim().length > 0 && name.trim().length > 0 && dir.trim().length > 0;

  return (
    <div className="modal-backdrop" onMouseDown={() => !running && onClose()}>
      <div className="modal" onMouseDown={(e) => e.stopPropagation()}>
        <div className="modal-title">Clone a repository</div>
        <label className="field">
          <span>Repository URL</span>
          <input
            autoFocus
            value={url}
            onChange={(e) => setUrl(e.target.value)}
            placeholder="https://github.com/owner/repo  ·  git@github.com:owner/repo.git  ·  owner/repo"
            spellCheck={false}
            disabled={running}
            onKeyDown={(e) => e.key === "Enter" && valid && !running && void start()}
          />
        </label>
        <label className="field">
          <span>Clone into</span>
          <div className="input-row">
            <input value={dir} onChange={(e) => setDir(e.target.value)} spellCheck={false} disabled={running} />
            <button className="btn" onClick={() => void browse()} disabled={running}>
              Browse
            </button>
          </div>
        </label>
        <label className="field">
          <span>Folder name</span>
          <input
            value={name}
            onChange={(e) => {
              setNameTouched(true);
              setName(e.target.value);
            }}
            spellCheck={false}
            disabled={running}
          />
        </label>
        {name && <div className="dim small mono">{dest}</div>}
        {log.length > 0 && (
          <pre className="clone-log" ref={logRef}>
            {log.join("\n")}
          </pre>
        )}
        {error && <pre className="commit-error">{error}</pre>}
        <div className="popover-actions">
          <button className="btn" onClick={onClose} disabled={running}>
            Cancel
          </button>
          <button className="btn primary" onClick={() => void start()} disabled={!valid || running}>
            {running ? "Cloning…" : "Clone"}
          </button>
        </div>
      </div>
    </div>
  );
}
