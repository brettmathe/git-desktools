// Self-update via tauri-plugin-updater: releases on GitHub publish a signed
// latest.json (see .github/workflows/release.yml); the app checks it, downloads
// and verifies the bundle, then relaunches on the user's say-so.
import { useSyncExternalStore } from "react";
import { getVersion } from "@tauri-apps/api/app";
import { relaunch } from "@tauri-apps/plugin-process";
import { check, type Update } from "@tauri-apps/plugin-updater";

export type UpdateStatus = "idle" | "checking" | "available" | "downloading" | "ready" | "uptodate" | "error";

export interface UpdateState {
  status: UpdateStatus;
  /** Version offered by the endpoint, once known. */
  version: string | null;
  currentVersion: string | null;
  notes: string | null;
  /** 0-1 while downloading; null when the server sends no content-length. */
  progress: number | null;
  error: string | null;
}

let state: UpdateState = {
  status: "idle",
  version: null,
  currentVersion: null,
  notes: null,
  progress: null,
  error: null,
};
let listeners: (() => void)[] = [];

function set(patch: Partial<UpdateState>) {
  state = { ...state, ...patch };
  listeners.forEach((l) => l());
}

export function useUpdater(): UpdateState {
  return useSyncExternalStore(
    (l) => {
      listeners.push(l);
      return () => {
        listeners = listeners.filter((x) => x !== l);
      };
    },
    () => state,
  );
}

/** Held between check and install so the user can decide in between. */
let pending: Update | null = null;

/** When the last check started (ms since epoch); 0 = never. Failed checks count,
 *  so an offline machine isn't retried on every focus. */
let lastCheckAt = 0;

export async function loadCurrentVersion() {
  try {
    set({ currentVersion: await getVersion() });
  } catch {
    // Display only (and absent under the dev bridge).
  }
}

/** The plugin throws this when the endpoint has no release JSON yet (a 404), which
 *  is the normal state before the first release, so it reads as up to date. */
const NO_RELEASE = "Could not fetch a valid release JSON from the remote";

/**
 * Ask the endpoint whether a newer build exists. `silent` is for automatic checks:
 * an offline machine or a dev build must not greet anyone with an error banner.
 */
export async function checkForUpdate(silent = false): Promise<UpdateStatus> {
  if (state.status === "checking" || state.status === "downloading") return state.status;
  lastCheckAt = Date.now();
  const before = state.status;
  set({ status: "checking", error: null });
  try {
    const update = await check();
    pending = update;
    if (update) {
      set({
        status: "available",
        version: update.version,
        currentVersion: update.currentVersion ?? state.currentVersion,
        notes: update.body ?? null,
      });
    } else {
      set({ status: "uptodate" });
    }
  } catch (e) {
    pending = null;
    const message = e instanceof Error ? e.message : String(e);
    if (message.includes(NO_RELEASE)) {
      set({ status: "uptodate" });
    } else if (silent) {
      console.warn("[git-desktools] update check failed:", e);
      set({ status: before === "uptodate" ? "uptodate" : "idle" });
    } else {
      set({ status: "error", error: message });
    }
  }
  return state.status;
}

/**
 * Silent background check, skipped when the last one is younger than `maxAgeMs`.
 * Once an update is offered, downloading or waiting for a restart, there is nothing
 * new to learn, and re-checking would reset that state.
 */
export async function checkForUpdateIfStale(maxAgeMs: number): Promise<void> {
  const s = state.status;
  if (s === "available" || s === "downloading" || s === "ready") return;
  if (Date.now() - lastCheckAt < maxAgeMs) return;
  await checkForUpdate(true);
}

/** Download, verify and install the pending update; the app then needs a restart. */
export async function installUpdate(): Promise<boolean> {
  if (!pending || state.status === "downloading") return false;
  set({ status: "downloading", progress: null, error: null });
  let total = 0;
  let received = 0;
  try {
    await pending.downloadAndInstall((event) => {
      if (event.event === "Started") {
        total = event.data.contentLength ?? 0;
        set({ progress: total > 0 ? 0 : null });
      } else if (event.event === "Progress") {
        received += event.data.chunkLength;
        if (total > 0) set({ progress: Math.min(received / total, 1) });
      } else if (event.event === "Finished") {
        set({ progress: 1 });
      }
    });
    set({ status: "ready" });
    return true;
  } catch (e) {
    set({ status: "error", error: e instanceof Error ? e.message : String(e) });
    return false;
  }
}

/** Restart into the freshly installed version. */
export async function restartApp() {
  await relaunch();
}
