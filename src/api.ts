import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  AutoBranched,
  BranchCheck,
  CommitDetail,
  CommitGuard,
  FileContent,
  FileDiff,
  RepoCheck,
  RepoData,
  RepoInfo,
  PushOutcome,
  RepoSummary,
  ScanHit,
  Settings,
  SmartEnv,
  SmartRepoStatus,
  WipStatus,
} from "./types";

export const api = {
  openRepo: (path: string) => invoke<RepoInfo>("open_repo", { path }),
  initRepo: (path: string) => invoke<RepoInfo>("init_repo", { path }),
  defaultCloneDir: () => invoke<string>("default_clone_dir"),
  repoSummary: (path: string) => invoke<RepoSummary>("repo_summary", { path }),
  repoFingerprint: (path: string) => invoke<string>("repo_fingerprint", { path }),
  loadRepo: (path: string) => invoke<RepoData>("load_repo", { path }),
  commitDetail: (path: string, oid: string) => invoke<CommitDetail>("commit_detail", { path, oid }),
  fileDiff: (path: string, oid: string, file: string, oldFile: string | null) =>
    invoke<FileDiff>("file_diff", { path, oid, file, oldFile }),
  wipFileDiff: (path: string, file: string, oldFile: string | null, staged: boolean) =>
    invoke<FileDiff>("wip_file_diff", { path, file, oldFile, staged }),
  fileContent: (path: string, oid: string | null, file: string) =>
    invoke<FileContent>("file_content", { path, oid, file }),
  wipStatus: (path: string) => invoke<WipStatus>("wip_status", { path }),
  stageFiles: (path: string, files: string[]) => invoke<void>("stage_files", { path, files }),
  unstageFiles: (path: string, files: string[]) => invoke<void>("unstage_files", { path, files }),
  headMessage: (path: string) => invoke<string>("head_message", { path }),
  commit: (path: string, message: string, amend: boolean) =>
    invoke<string>("commit", { path, message, amend }),
  cloneRepo: (url: string, parentDir: string, name: string) =>
    invoke<RepoInfo>("clone_repo", { url, parentDir, name }),
  fetchRepo: (path: string, quiet: boolean) =>
    invoke<{ skipped: boolean }>("fetch_repo", { path, quiet }),
  pullRepo: (path: string) =>
    invoke<{ count: number; upstream: string | null }>("pull_repo", { path }),
  pushRepo: (path: string, remote: string | null, branch: string | null) =>
    invoke<{ upstream: string | null }>("push_repo", { path, remote, branch }),
  ptySpawn: (cwd: string, kind: "claude" | "shell", cols: number, rows: number) =>
    invoke<number>("pty_spawn", { cwd, kind, cols, rows }),
  ptyWrite: (id: number, data: string) => invoke<void>("pty_write", { id, data }),
  ptyResize: (id: number, cols: number, rows: number) => invoke<void>("pty_resize", { id, cols, rows }),
  ptyKill: (id: number) => invoke<void>("pty_kill", { id }),
  loadSettings: () => invoke<Partial<Settings> | null>("load_settings"),
  saveSettings: (settings: Settings) => invoke<void>("save_settings", { settings }),
  sfListEnvs: () => invoke<{ envs: SmartEnv[]; warnings: string[] }>("sf_list_envs"),
  sfSaveEnv: (env: SmartEnv) => invoke<SmartEnv>("sf_save_env", { env }),
  sfDeleteEnv: (id: string) => invoke<void>("sf_delete_env", { id }),
  sfValidateRepo: (path: string, envId: string | null) =>
    invoke<RepoCheck>("sf_validate_repo", { path, envId }),
  sfScanFolder: (path: string, envId: string | null) => invoke<ScanHit[]>("sf_scan_folder", { path, envId }),
  sfStatus: (envId: string) => invoke<SmartRepoStatus[]>("sf_status", { envId }),
  sfCheckBranch: (envId: string, branch: string) => invoke<BranchCheck>("sf_check_branch", { envId, branch }),
  sfStartFeature: (envId: string, name: string, branch: string, adopt: boolean) =>
    invoke<{ env: SmartEnv; moved: AutoBranched[]; warnings: string[] }>("sf_start_feature", {
      envId,
      name,
      branch,
      adopt,
    }),
  sfBackToBase: (envId: string) => invoke<SmartEnv>("sf_back_to_base", { envId }),
  sfMoveToFeature: (envId: string, repoId: string) =>
    invoke<AutoBranched | null>("sf_move_to_feature", { envId, repoId }),
  sfCommitGuard: (path: string) => invoke<CommitGuard | null>("sf_commit_guard", { path }),
  sfCheckoutRemote: (envId: string, repoId: string) => invoke<SmartEnv>("sf_checkout_remote", { envId, repoId }),
  sfRenameFeature: (envId: string, featureId: string, name: string) =>
    invoke<SmartEnv>("sf_rename_feature", { envId, featureId, name }),
  sfPushAll: (envId: string, featureId: string) => invoke<PushOutcome[]>("sf_push_all", { envId, featureId }),
};

/** Backend Smart Features events (see smart_watch.rs). */
export function onSmartEvents(handlers: {
  repoChanged: (p: { envId: string; repoId: string }) => void;
  autoBranched: (p: AutoBranched) => void;
  autoBranchFailed: (p: { envId: string; repoId: string; error: string }) => void;
}): () => void {
  const subs = [
    listen<{ envId: string; repoId: string }>("sf-repo-changed", (e) => handlers.repoChanged(e.payload)),
    listen<AutoBranched>("sf-auto-branched", (e) => handlers.autoBranched(e.payload)),
    listen<{ envId: string; repoId: string; error: string }>("sf-auto-branch-failed", (e) =>
      handlers.autoBranchFailed(e.payload),
    ),
  ];
  return () => subs.forEach((s) => void s.then((un) => un()));
}

export function errorText(err: unknown): string {
  return err instanceof Error ? err.message : String(err);
}

export function onProgress(
  event: "clone-progress" | "pull-progress" | "push-progress",
  key: string,
  cb: (line: string) => void,
): Promise<UnlistenFn> {
  return listen<{ key: string; line: string }>(event, (e) => {
    if (e.payload.key === key) cb(e.payload.line);
  });
}

// ---------------------------------------------------------------------------
// Terminal output bus. One global listener; output that arrives before a view
// attaches (it can beat the pty_spawn response) is buffered.

interface PtyHandler {
  data: (bytes: Uint8Array) => void;
  exit: () => void;
}

const handlers = new Map<number, PtyHandler>();
const pending = new Map<number, Uint8Array[]>();
const exited = new Set<number>();
let busReady: Promise<void> | null = null;

function decode(b64: string): Uint8Array {
  const bin = atob(b64);
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out;
}

export function initPtyBus(): Promise<void> {
  busReady ??= Promise.all([
    listen<{ id: number; data: string }>("pty-data", ({ payload }) => {
      const bytes = decode(payload.data);
      const h = handlers.get(payload.id);
      if (h) h.data(bytes);
      else pending.set(payload.id, [...(pending.get(payload.id) ?? []), bytes]);
    }),
    listen<{ id: number }>("pty-exit", ({ payload }) => {
      const h = handlers.get(payload.id);
      if (h) h.exit();
      else exited.add(payload.id);
    }),
  ]).then(() => undefined);
  return busReady;
}

export function attachPty(id: number, handler: PtyHandler): () => void {
  handlers.set(id, handler);
  for (const chunk of pending.get(id) ?? []) handler.data(chunk);
  pending.delete(id);
  if (exited.delete(id)) handler.exit();
  return () => {
    handlers.delete(id);
  };
}
