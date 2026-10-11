import { WIP_COLOR, type FileChange } from "./types";

export const LANE_COLORS = [
  "#15a0bf",
  "#0669f7",
  "#8e00c2",
  "#c517b6",
  "#d90171",
  "#cd0101",
  "#f25d2e",
  "#f2ca33",
  "#7bd938",
  "#2ece9d",
];
export const WIP_STROKE = "#9aa1ad";

export function laneColor(c: number): string {
  return c === WIP_COLOR ? WIP_STROKE : LANE_COLORS[c % LANE_COLORS.length];
}

export const isMac = navigator.userAgent.includes("Mac");

export function shortSha(id: string): string {
  return id.slice(0, 7);
}

const DAY = 86400;

/** Bucket used for the date separators in the commit list. */
export function dateBucket(time: number, now: number): string {
  const days = Math.floor((now - time) / DAY);
  if (days < 1) return "today";
  if (days < 2) return "yesterday";
  if (days < 7) return `${days} days ago`;
  if (days < 30) {
    const w = Math.floor(days / 7);
    return w === 1 ? "a week ago" : `${w} weeks ago`;
  }
  if (days < 365) {
    const m = Math.floor(days / 30);
    return m === 1 ? "a month ago" : `${m} months ago`;
  }
  const y = Math.floor(days / 365);
  return y === 1 ? "a year ago" : `${y} years ago`;
}

export function relativeTime(ms: number): string {
  const s = Math.round((Date.now() - ms) / 1000);
  if (s < 5) return "just now";
  if (s < 60) return `${s}s ago`;
  const m = Math.round(s / 60);
  if (m < 60) return `${m}m ago`;
  return `${Math.round(m / 60)}h ago`;
}

export function formatDate(time: number): string {
  const d = new Date(time * 1000);
  const date = d.toLocaleDateString(undefined, { month: "2-digit", day: "2-digit", year: "numeric" });
  const clock = d.toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" });
  return `${date} @ ${clock}`;
}

export function hashColor(s: string): string {
  let h = 0;
  for (let i = 0; i < s.length; i++) h = (h * 31 + s.charCodeAt(i)) | 0;
  return `hsl(${Math.abs(h) % 360} 45% 42%)`;
}

export function splitPath(path: string): [string, string] {
  const i = path.lastIndexOf("/");
  return i < 0 ? ["", path] : [path.slice(0, i + 1), path.slice(i + 1)];
}

export function plural(n: number, word: string): string {
  return `${n} ${word}${n === 1 ? "" : "s"}`;
}

export interface FileStats {
  modified: number;
  added: number;
  deleted: number;
  renamed: number;
}

export function fileStats(files: FileChange[]): FileStats {
  const s = { modified: 0, added: 0, deleted: 0, renamed: 0 };
  for (const f of files) {
    if (f.status === "A" || f.status === "C") s.added++;
    else if (f.status === "D") s.deleted++;
    else if (f.status === "R") s.renamed++;
    else s.modified++;
  }
  return s;
}

/** Nested folder structure for branch names or file paths split on "/". */
export interface TreeNode<T> {
  name: string;
  key: string;
  children: TreeNode<T>[];
  item?: T;
}

export function buildTree<T>(items: T[], pathOf: (t: T) => string): TreeNode<T>[] {
  const root: TreeNode<T> = { name: "", key: "", children: [] };
  for (const item of items) {
    const parts = pathOf(item).split("/");
    let node = root;
    for (let i = 0; i < parts.length - 1; i++) {
      let folder = node.children.find((c) => !c.item && c.name === parts[i]);
      if (!folder) {
        folder = { name: parts[i], key: parts.slice(0, i + 1).join("/") + "/", children: [] };
        node.children.push(folder);
      }
      node = folder;
    }
    node.children.push({ name: parts[parts.length - 1], key: pathOf(item), children: [], item });
  }
  const sort = (nodes: TreeNode<T>[]) => {
    nodes.sort((a, b) => Number(!!a.item) - Number(!!b.item) || a.name.localeCompare(b.name));
    nodes.forEach((n) => sort(n.children));
  };
  sort(root.children);
  return root.children;
}

/** github.com/o/r, o/r, git@… → a clone URL plus a default folder name. */
export function normalizeCloneUrl(input: string): { url: string; name: string } {
  let url = input.trim();
  if (/^[\w.-]+\/[\w.-]+$/.test(url)) url = `https://github.com/${url}.git`;
  else if (/^github\.com\//.test(url)) url = `https://${url}`;
  const name = url
    .replace(/\/+$/, "")
    .replace(/\.git$/, "")
    .split(/[/:]/)
    .pop() ?? "";
  return { url, name };
}
