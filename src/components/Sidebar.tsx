import { useMemo, useState, type ReactNode } from "react";
import type { RepoData } from "../types";
import { buildTree, type TreeNode } from "../util";
import {
  BranchIcon,
  CheckIcon,
  ChevronIcon,
  ChevronLeftIcon,
  CloudIcon,
  FolderIcon,
  GitHubIcon,
  LaptopIcon,
  SearchIcon,
  TagIcon,
} from "./icons";

interface Props {
  data: RepoData;
  onJump: (oid: string) => void;
  onCollapse: () => void;
}

type SectionKey = "local" | "remote" | "tags";

export function Sidebar({ data, onJump, onCollapse }: Props) {
  const [filter, setFilter] = useState("");
  const [open, setOpen] = useState<Record<SectionKey, boolean>>({ local: true, remote: true, tags: false });
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());
  const q = filter.trim().toLowerCase();
  const filtering = q.length > 0;
  const match = (s: string) => !filtering || s.toLowerCase().includes(q);

  const locals = data.locals.filter((b) => match(b.name));
  const remotes = data.remotes.filter((b) => match(b.fullName));
  const tags = data.tags
    .filter((t) => match(t.name))
    .sort((a, b) => b.name.localeCompare(a.name, undefined, { numeric: true }));

  const remoteGroups = useMemo(() => {
    const groups = new Map<string, typeof remotes>();
    for (const r of remotes) groups.set(r.remote, [...(groups.get(r.remote) ?? []), r]);
    return [...groups.entries()].map(([remote, items]) => ({
      remote,
      url: data.remoteInfos.find((i) => i.name === remote)?.url ?? "",
      tree: buildTree(items, (i) => i.name),
    }));
  }, [remotes, data.remoteInfos]);

  const toggle = (key: string) =>
    setCollapsed((s) => {
      const next = new Set(s);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  const isOpen = (key: string) => filtering || !collapsed.has(key);

  const renderTree = (nodes: TreeNode<{ target: string; fullName: string }>[], depth: number, prefix: string): ReactNode =>
    nodes.map((n) =>
      n.item ? (
        <div
          key={n.key}
          className="sb-row"
          style={{ paddingLeft: 12 + depth * 14 }}
          onClick={() => onJump(n.item!.target)}
          title={n.item.fullName}
        >
          <BranchIcon size={13} className="dim" />
          <span className="sb-name">{n.name}</span>
        </div>
      ) : (
        <div key={n.key}>
          <div className="sb-row folder" style={{ paddingLeft: 12 + depth * 14 }} onClick={() => toggle(prefix + n.key)}>
            <FolderIcon size={13} className="dim" />
            <span className="sb-name">{n.name}</span>
          </div>
          {isOpen(prefix + n.key) && renderTree(n.children, depth + 1, prefix)}
        </div>
      ),
    );

  const section = (key: SectionKey, label: string, icon: ReactNode, count: number, body: ReactNode) => (
    <div className="sb-section">
      <div className="sb-header" onClick={() => setOpen((o) => ({ ...o, [key]: !o[key] }))}>
        <ChevronIcon size={12} open={open[key] || filtering} />
        {icon}
        <span>{label}</span>
        <span className="sb-count">{count}</span>
      </div>
      {(open[key] || filtering) && <div className="sb-body">{body}</div>}
    </div>
  );

  return (
    <aside className="sidebar">
      <div className="sb-top">
        <button className="icon-btn round" onClick={onCollapse} title="Hide sidebar">
          <ChevronLeftIcon size={14} />
        </button>
        <span className="sb-viewing">
          Viewing <b>{data.locals.length + data.remotes.length + data.tags.length}</b>
        </span>
      </div>
      <div className="sb-filter">
        <input value={filter} onChange={(e) => setFilter(e.target.value)} placeholder="Filter branches and tags" spellCheck={false} />
        <SearchIcon size={13} />
      </div>
      <div className="sb-scroll">
        {section(
          "local",
          "Local",
          <LaptopIcon size={14} />,
          data.locals.length,
          locals.length === 0 ? (
            <div className="sb-empty">{filtering ? "No matches" : data.head.unborn ? `${data.head.branch ?? "main"} (no commits yet)` : "No local branches"}</div>
          ) : (
            locals.map((b) => (
              <div
                key={b.name}
                className={`sb-row${b.isHead ? " head" : ""}`}
                onClick={() => onJump(b.target)}
                title={b.upstream ? `${b.name} → ${b.upstream}` : b.name}
              >
                {b.isHead ? (
                  <span className="sb-check">
                    <CheckIcon size={11} />
                  </span>
                ) : (
                  <BranchIcon size={13} className="dim" />
                )}
                <span className="sb-name">{b.name}</span>
                <span className="sb-ab">
                  {b.ahead > 0 && <span>{b.ahead}↑</span>}
                  {b.behind > 0 && <span>{b.behind}↓</span>}
                </span>
              </div>
            ))
          ),
        )}
        {section(
          "remote",
          "Remote",
          <CloudIcon size={14} />,
          data.remotes.length,
          remoteGroups.length === 0 ? (
            <div className="sb-empty">{filtering ? "No matches" : "No remotes"}</div>
          ) : (
            remoteGroups.map((g) => (
              <div key={g.remote}>
                <div className="sb-row folder" style={{ paddingLeft: 12 }} onClick={() => toggle(`remote:${g.remote}`)} title={g.url}>
                  {g.url.includes("github.com") ? <GitHubIcon size={13} /> : <CloudIcon size={13} />}
                  <span className="sb-name">{g.remote}</span>
                </div>
                {isOpen(`remote:${g.remote}`) && renderTree(g.tree, 1, `remote:${g.remote}:`)}
              </div>
            ))
          ),
        )}
        {section(
          "tags",
          "Tags",
          <TagIcon size={14} />,
          data.tags.length,
          tags.length === 0 ? (
            <div className="sb-empty">{filtering ? "No matches" : "No tags"}</div>
          ) : (
            tags.map((t) => (
              <div key={t.name} className="sb-row" style={{ paddingLeft: 26 }} onClick={() => onJump(t.target)}>
                <TagIcon size={13} className="dim" />
                <span className="sb-name">{t.name}</span>
              </div>
            ))
          ),
        )}
      </div>
    </aside>
  );
}
