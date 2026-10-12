/** [fromLane, fromY, toLane, toY, color]; y: 0 = top, 1 = middle, 2 = bottom of the row. */
export type Seg = [number, number, number, number, number];

export const WIP_ID = "WIP";
export const WIP_COLOR = 65535;

export interface RepoInfo {
  root: string;
  name: string;
}

export interface RepoSummary extends RepoInfo {
  branch: string | null;
  ahead: number;
  behind: number;
  owner: string | null;
}

export interface RefLabel {
  name: string;
  kind: "local" | "remote" | "tag";
  remote: string | null;
  isHead: boolean;
}

export interface CommitRow {
  id: string;
  summary: string;
  body: string;
  author: number;
  time: number;
  merge: boolean;
  lane: number;
  color: number;
  segs: Seg[];
  refs: RefLabel[];
}

export interface Author {
  name: string;
  email: string;
  gravatar: string;
}

export interface LocalBranch {
  name: string;
  target: string;
  upstream: string | null;
  ahead: number;
  behind: number;
  isHead: boolean;
}

export interface RemoteBranch {
  remote: string;
  name: string;
  fullName: string;
  target: string;
}

export interface TagInfo {
  name: string;
  target: string;
}

export interface RemoteInfo {
  name: string;
  url: string | null;
}

export interface HeadInfo {
  branch: string | null;
  oid: string | null;
  detached: boolean;
  unborn: boolean;
}

export interface RepoData {
  root: string;
  name: string;
  fingerprint: string;
  truncated: boolean;
  hasWip: boolean;
  head: HeadInfo;
  authors: Author[];
  commits: CommitRow[];
  maxLanes: number;
  locals: LocalBranch[];
  remotes: RemoteBranch[];
  remoteInfos: RemoteInfo[];
  tags: TagInfo[];
}

export interface Person {
  name: string;
  email: string;
  gravatar: string;
  time: number;
}

export interface FileChange {
  path: string;
  oldPath: string | null;
  status: "A" | "M" | "D" | "R" | "C" | "T" | "U";
  additions: number;
  deletions: number;
  binary: boolean;
}

export interface CommitDetail {
  id: string;
  summary: string;
  body: string;
  author: Person;
  committer: Person;
  parents: string[];
  coAuthors: { name: string; email: string; gravatar: string }[];
  files: FileChange[];
  filesTruncated: boolean;
}

export interface DiffLine {
  o: "+" | "-" | " " | "\\";
  a: number | null;
  b: number | null;
  t: string;
}

export interface FileDiff {
  binary: boolean;
  truncated: boolean;
  hunks: { header: string; lines: DiffLine[] }[];
}

export interface FileContent {
  binary: boolean;
  truncated: boolean;
  content: string;
}

export interface WipStatus {
  unstaged: FileChange[];
  staged: FileChange[];
}

export type OpenFile =
  | { kind: "commit"; commit: string; file: FileChange }
  | { kind: "wip"; staged: boolean; file: FileChange };

export interface RecentRepo {
  path: string;
  name: string;
  lastOpened: number;
}

export interface Settings {
  recent: RecentRepo[];
  openTabs: string[];
  activeTab: string | null;
  cloneDir?: string;
  autoFetchSeconds: number;
  smartFeatures?: SmartFeaturesUi;
}

/** Panel UI state only; environments themselves are stored by the backend. */
export interface SmartFeaturesUi {
  panelOpen: boolean;
  width: number;
  collapsed: string[];
  changedOnly: boolean;
}

// ---------------------------------------------------------------------------
// Smart Environments / Smart Features

export interface SmartRepo {
  id: string;
  path: string;
  label: string;
  /** null = resolve from origin/HEAD, then develop/main/master. */
  baseBranch: string | null;
}

export interface SmartFeature {
  id: string;
  name: string;
  branch: string;
  createdAt: number;
  repos: Record<string, { parkedStash: { ref: string; sha: string; parkedAt: number } | null }>;
}

export interface SmartEnv {
  version: number;
  id: string;
  name: string;
  settings: { sharedBase: string | null };
  repos: SmartRepo[];
  activeFeatureId: string | null;
  features: SmartFeature[];
  /** An unfinished feature switch; null when idle. */
  journal: SwitchJournal | null;
}

export interface SwitchStep {
  repoId: string;
  label: string;
  action: "park" | "base" | "switch" | "restore";
  featureId: string | null;
  /** park: "changes" when there's work to stash; base/switch: the branch. */
  detail: string | null;
  status: "pending" | "done" | "failed" | "warning";
  message: string | null;
}

export interface SwitchJournal {
  op: string;
  from: string | null;
  to: string | null;
  startedAt: number;
  steps: SwitchStep[];
}

export interface Preflight {
  from: string | null;
  to: string | null;
  problems: string[];
  steps: SwitchStep[];
}

export interface SwitchResult {
  env: SmartEnv;
  warnings: string[];
}

export interface FinishRepo {
  repoId: string;
  label: string;
  local: boolean;
  hasUpstream: boolean;
  unpushed: number;
  merged: boolean;
  parked: boolean;
  atRisk: boolean;
}

export type Dot = "red" | "yellow" | "green" | "none";

export interface SmartRepoStatus {
  repoId: string;
  path: string;
  label: string;
  missing: boolean;
  branch: string | null;
  detached: boolean;
  baseBranch: string | null;
  expectedBranch: string | null;
  staged: number;
  unstaged: number;
  untracked: number;
  conflicted: number;
  hasUpstream: boolean;
  ahead: number;
  behind: number;
  aheadOfBase: number;
  inProgress: string | null;
  dot: Dot;
  warning: string | null;
  /** Where each feature's branch exists in this repo. */
  features: FeatureRef[];
}

export interface FeatureRef {
  featureId: string;
  local: boolean;
  /** origin/<branch> exists. */
  remote: boolean;
  hasUpstream: boolean;
  /** Commits not on the upstream (or, with no upstream, not on base). */
  unpushed: number;
  aheadOfBase: number;
  /** Parked (pinned) uncommitted changes exist here. */
  parked: boolean;
}

export interface BranchCheck {
  error: string | null;
  repos: {
    repoId: string;
    label: string;
    local: boolean;
    remote: boolean;
    dirtyOnBase: boolean;
    conflict: string | null;
  }[];
}

export interface AutoBranched {
  envId: string;
  repoId: string;
  label: string;
  branch: string;
  created: boolean;
}

export interface CommitGuard {
  envId: string;
  envName: string;
  repoId: string;
  featureName: string;
  branch: string;
  base: string;
}

export interface PushOutcome {
  repoId: string;
  label: string;
  status: "pushed" | "skipped" | "failed";
  detail: string;
}

export interface RepoCheck {
  root: string;
  name: string;
  branch: string | null;
  baseBranch: string | null;
  inEnv: string | null;
}

export interface ScanHit {
  root: string;
  name: string;
  inEnv: string | null;
}

export type FileViewMode = "path" | "tree";
