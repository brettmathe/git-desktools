// Wording for a merge, rebase, etc. that is in progress (see RepoOp).
import type { OpKind, OpSide, RepoOp } from "./types";
import { plural } from "./util";

const NOUN: Record<OpKind, string> = {
  merge: "Merge",
  "cherry-pick": "Cherry-pick",
  revert: "Revert",
  rebase: "Rebase",
  am: "Patch",
  stash: "Stash",
};

const short = (oid: string | null) => oid?.slice(0, 7) ?? "";

/** The branch the operation lands on. */
export function opTarget(op: RepoOp): string {
  return op.branch ?? (op.ours.oid ? `${short(op.ours.oid)} (detached)` : "HEAD");
}

/** Right-panel heading. */
export function opTitle(op: RepoOp): string {
  return op.conflicts.length > 0 ? `${NOUN[op.kind]} conflicts detected` : `${NOUN[op.kind]} in progress`;
}

/** The amber text on the WIP row of the graph. */
export function opNote(op: RepoOp): string {
  const n = op.conflicts.length;
  const target = opTarget(op);
  const when = {
    merge: `when attempting to merge into ${target}`,
    "cherry-pick": `when cherry-picking ${short(op.theirs.oid)} onto ${target}`,
    revert: `when reverting ${short(op.theirs.oid)} on ${target}`,
    rebase: `while rebasing ${target} onto ${op.ours.label}`,
    am: `while applying patches to ${target}`,
    stash: `while applying a stash to ${target}`,
  }[op.kind];
  if (n > 0) return `${plural(n, "file conflict")} ${n === 1 ? "was" : "were"} found ${when}`;
  if (op.kind === "stash") return "Stash conflicts resolved";
  return `Conflicts resolved: ${op.kind === "merge" ? "commit to finish the merge" : `continue the ${NOUN[op.kind].toLowerCase()}`}`;
}

/** "Commit 322a3f on main", or the commit label itself ("5b93d3 Fix greeting"). */
export function sideTitle(side: OpSide): string {
  if (!side.oid) return side.label;
  const s = short(side.oid);
  return side.label.startsWith(s) ? `Commit ${side.label}` : `Commit ${s} on ${side.label}`;
}

/** Commit-box button label for finishing the operation. */
export function continueLabel(op: RepoOp): string {
  switch (op.kind) {
    case "merge":
      return "Commit and Merge";
    case "am":
      return "Continue Applying Patches";
    default:
      return `Continue ${NOUN[op.kind]}`;
  }
}

export function abortLabel(op: RepoOp): string {
  return op.kind === "stash" ? "Undo Stash Apply" : op.kind === "am" ? "Abort Patches" : `Abort ${NOUN[op.kind]}`;
}

/** What Abort does, for its confirmation. */
export function abortDetail(op: RepoOp): string {
  const target = opTarget(op);
  switch (op.kind) {
    case "stash":
      return "The stash's changes are taken back out of your working tree. The stash entry is kept, so you can apply it again.";
    case "rebase":
      return `${target} goes back to where it was before the rebase started. Commits already replayed and any resolutions are dropped.`;
    default:
      return `${target} goes back to how it was before the ${NOUN[op.kind].toLowerCase()} started. Resolutions you've made are lost.`;
  }
}

/** Identifies one stop of an operation, so drafts reset when it moves on. */
export function opKey(op: RepoOp): string {
  return `${op.kind}:${op.theirs.oid ?? op.theirs.label}:${op.step ?? ""}`;
}
