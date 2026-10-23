import type { RefLabel } from "./types";

/** One branch name on a commit: its local branch and/or remote branches, or a tag. */
export interface Pill {
  name: string;
  local: RefLabel | null;
  remotes: RefLabel[];
  tag: boolean;
}

export function toPills(refs: RefLabel[]): Pill[] {
  const pills: Pill[] = [];
  for (const r of refs) {
    if (r.kind === "local") pills.push({ name: r.name, local: r, remotes: [], tag: false });
    else if (r.kind === "remote") {
      const same = pills.find((p) => p.local && p.name === r.name);
      if (same) same.remotes.push(r);
      else pills.push({ name: r.name, local: null, remotes: [r], tag: false });
    } else pills.push({ name: r.name, local: null, remotes: [], tag: true });
  }
  return pills;
}

/**
 * The branch a double-click on a commit checks out: its first local branch
 * that isn't already checked out, else a remote branch with no local branch
 * on this commit (switching to that name keeps an existing local branch where it is).
 */
export function checkoutTarget(refs: RefLabel[]): { branch: string; track: string | null } | null {
  for (const p of toPills(refs)) {
    if (p.local) {
      if (!p.local.isHead) return { branch: p.name, track: null };
    } else if (p.remotes.length && p.name !== "HEAD") {
      const r = p.remotes[0];
      return { branch: p.name, track: `${r.remote}/${r.name}` };
    }
  }
  return null;
}
