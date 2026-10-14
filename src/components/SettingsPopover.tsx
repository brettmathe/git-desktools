import { useState } from "react";
import { showToast } from "../toast";
import type { Settings } from "../types";
import { checkForUpdate, installUpdate, restartApp, useUpdater } from "../updater";

interface Props {
  settings: Settings;
  onChange: (patch: Partial<Settings>) => void;
  onClose: () => void;
}

export function SettingsPopover({ settings, onChange, onClose }: Props) {
  const enabled = settings.autoFetchSeconds > 0;
  const [seconds, setSeconds] = useState(String(enabled ? settings.autoFetchSeconds : 60));

  const apply = (on: boolean, value: string) => {
    const n = Math.max(15, Math.round(Number(value) || 60));
    onChange({ autoFetchSeconds: on ? n : 0 });
  };

  return (
    <>
      <div className="backdrop" onMouseDown={onClose} />
      <div className="popover settings-popover">
        <div className="popover-title">Settings</div>
        <label className="check-row">
          <input type="checkbox" checked={enabled} onChange={(e) => apply(e.target.checked, seconds)} />
          Fetch remotes in the background
        </label>
        <label className="field inline">
          <span>Every</span>
          <input
            type="number"
            min={15}
            step={15}
            value={seconds}
            disabled={!enabled}
            onChange={(e) => setSeconds(e.target.value)}
            onBlur={() => apply(enabled, seconds)}
          />
          <span>seconds</span>
        </label>
        <p className="dim small">Background fetches never pull or change your working tree.</p>
        <UpdatesSection />
      </div>
    </>
  );
}

function UpdatesSection() {
  const u = useUpdater();
  const manualCheck = async () => {
    if ((await checkForUpdate()) === "uptodate") showToast("git-desktools is up to date.", "success");
  };
  return (
    <div className="settings-section">
      <div className="popover-title">Updates</div>
      <div className="update-row">
        <span className="dim">Version {u.currentVersion ?? "…"}</span>
        {u.status === "available" ? (
          <button className="btn small primary" onClick={() => void installUpdate()}>
            Install {u.version}
          </button>
        ) : u.status === "downloading" ? (
          <button className="btn small" disabled>
            {u.progress === null ? "Downloading…" : `Downloading ${Math.round(u.progress * 100)}%`}
          </button>
        ) : u.status === "ready" ? (
          <button className="btn small primary" onClick={() => void restartApp()}>
            Restart to finish
          </button>
        ) : (
          <button className="btn small" onClick={() => void manualCheck()} disabled={u.status === "checking"}>
            {u.status === "checking" ? "Checking…" : "Check for updates"}
          </button>
        )}
      </div>
      {u.status === "error" && u.error && <p className="error-text small">{u.error}</p>}
      {u.status === "available" && u.notes && <p className="dim small">{u.notes}</p>}
      <p className="dim small">Checked on launch and every few hours. Updates are signed; unsigned builds are refused.</p>
    </div>
  );
}
