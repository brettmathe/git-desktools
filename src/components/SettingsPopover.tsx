import { useState } from "react";
import type { Settings } from "../types";

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
      </div>
    </>
  );
}
