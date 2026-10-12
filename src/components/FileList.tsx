import { useEffect, useState, type MouseEvent, type ReactNode } from "react";
import type { FileChange, FileViewMode } from "../types";
import { buildTree, splitPath, type TreeNode } from "../util";
import { FolderIcon, ListIcon, StatusIcon, TreeIcon } from "./icons";

export interface MenuItem {
  label: string;
  onClick: () => void;
  danger?: boolean;
  disabled?: boolean;
}

interface Props {
  files: FileChange[];
  mode: FileViewMode;
  selectedPath: string | null;
  onOpen: (file: FileChange) => void;
  action?: (file: FileChange) => ReactNode;
  showStats?: boolean;
  /** Right-click menu for a file, or for a folder in tree view (`files` = everything under it). */
  contextMenu?: (target: { file: FileChange } | { folder: string; files: FileChange[] }) => MenuItem[];
}

/** Files under a tree folder node. */
function filesUnder(n: TreeNode<FileChange>): FileChange[] {
  return n.item ? [n.item] : n.children.flatMap(filesUnder);
}

export function FileList({ files, mode, selectedPath, onOpen, action, showStats, contextMenu }: Props) {
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());
  const [menu, setMenu] = useState<{ x: number; y: number; items: MenuItem[] } | null>(null);

  useEffect(() => {
    if (!menu) return;
    const close = () => setMenu(null);
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && close();
    window.addEventListener("keydown", onKey);
    window.addEventListener("blur", close);
    window.addEventListener("resize", close);
    document.addEventListener("scroll", close, true);
    return () => {
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("blur", close);
      window.removeEventListener("resize", close);
      document.removeEventListener("scroll", close, true);
    };
  }, [menu]);

  const openMenu = (e: MouseEvent, items: MenuItem[] | undefined) => {
    if (!items?.length) return;
    e.preventDefault();
    e.stopPropagation();
    // Keep it on screen.
    const x = Math.min(e.clientX, window.innerWidth - 230);
    const y = Math.min(e.clientY, window.innerHeight - (items.length * 30 + 12));
    setMenu({ x, y, items });
  };

  const row = (f: FileChange, label: ReactNode, indent: number) => (
    <div
      key={f.path}
      className={`file-row${f.path === selectedPath ? " selected" : ""}`}
      style={{ paddingLeft: 10 + indent }}
      onClick={() => onOpen(f)}
      onContextMenu={(e) => openMenu(e, contextMenu?.({ file: f }))}
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

  const menuView = menu && (
    <>
      <div
        className="backdrop"
        onMouseDown={() => setMenu(null)}
        onContextMenu={(e) => {
          e.preventDefault();
          setMenu(null);
        }}
      />
      <div className="context-menu" style={{ left: menu.x, top: menu.y }} role="menu">
        {menu.items.map((item) => (
          <button
            key={item.label}
            role="menuitem"
            className={item.danger ? "danger" : undefined}
            disabled={item.disabled}
            onClick={() => {
              setMenu(null);
              item.onClick();
            }}
          >
            {item.label}
          </button>
        ))}
      </div>
    </>
  );

  if (mode === "path") {
    return (
      <div className="file-list">
        {menuView}
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
            onContextMenu={(e) => openMenu(e, contextMenu?.({ folder: n.key, files: filesUnder(n) }))}
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

  return (
    <div className="file-list">
      {menuView}
      {render(buildTree(files, (f) => f.path), 0)}
    </div>
  );
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
