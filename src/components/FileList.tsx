import { useState, type ReactNode } from "react";
import type { FileChange, FileViewMode } from "../types";
import { buildTree, splitPath, type TreeNode } from "../util";
import { FolderIcon, ListIcon, StatusIcon, TreeIcon } from "./icons";

interface Props {
  files: FileChange[];
  mode: FileViewMode;
  selectedPath: string | null;
  onOpen: (file: FileChange) => void;
  action?: (file: FileChange) => ReactNode;
  showStats?: boolean;
}

export function FileList({ files, mode, selectedPath, onOpen, action, showStats }: Props) {
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());

  const row = (f: FileChange, label: ReactNode, indent: number) => (
    <div
      key={f.path}
      className={`file-row${f.path === selectedPath ? " selected" : ""}`}
      style={{ paddingLeft: 10 + indent }}
      onClick={() => onOpen(f)}
      title={f.oldPath ? `${f.oldPath} → ${f.path}` : f.path}
    >
      <StatusIcon status={f.status} />
      {label}
      {showStats && !f.binary && (f.additions > 0 || f.deletions > 0) && (
        <span className="file-stats">
          {f.additions > 0 && <span className="st-added">+{f.additions}</span>}
          {f.deletions > 0 && <span className="st-deleted">−{f.deletions}</span>}
        </span>
      )}
      {action && <span className="file-action">{action(f)}</span>}
    </div>
  );

  if (mode === "path") {
    return (
      <div className="file-list">
        {files.map((f) => {
          const [dir, base] = splitPath(f.path);
          return row(
            f,
            <span className="file-path">
              <span className="file-dir">{dir}</span>
              <span className="file-base">{base}</span>
            </span>,
            0,
          );
        })}
      </div>
    );
  }

  const render = (nodes: TreeNode<FileChange>[], depth: number): ReactNode =>
    nodes.map((n) =>
      n.item ? (
        row(n.item, <span className="file-base">{n.name}</span>, depth * 14)
      ) : (
        <div key={n.key}>
          <div
            className="file-row folder"
            style={{ paddingLeft: 10 + depth * 14 }}
            onClick={() =>
              setCollapsed((s) => {
                const next = new Set(s);
                if (next.has(n.key)) next.delete(n.key);
                else next.add(n.key);
                return next;
              })
            }
          >
            <FolderIcon size={13} className="dim" />
            <span className="file-base">{n.name}</span>
          </div>
          {!collapsed.has(n.key) && render(n.children, depth + 1)}
        </div>
      ),
    );

  return <div className="file-list">{render(buildTree(files, (f) => f.path), 0)}</div>;
}

export function ViewModeToggle({ mode, onChange }: { mode: FileViewMode; onChange: (m: FileViewMode) => void }) {
  return (
    <div className="seg-toggle">
      <button className={mode === "path" ? "on" : ""} onClick={() => onChange("path")}>
        <ListIcon size={13} /> Path
      </button>
      <button className={mode === "tree" ? "on" : ""} onClick={() => onChange("tree")}>
        <TreeIcon size={13} /> Tree
      </button>
    </div>
  );
}
