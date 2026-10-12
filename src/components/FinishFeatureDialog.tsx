import { useEffect, useState } from "react";
import { api, errorText } from "../api";
import type { FinishRepo, SmartEnv, SmartFeature, SwitchResult } from "../types";

interface Props {
  env: SmartEnv;
  feature: SmartFeature;
  onClose: () => void;
  onDone: (result: SwitchResult | null) => void;
}

function state(r: FinishRepo): string {
  const parts: string[] = [];
  if (!r.local) parts.push("no local branch");
  else if (!r.hasUpstream && r.unpushed === 0) parts.push("no commits of its own");
  else if (r.merged) parts.push("merged");
  else if (r.unpushed > 0) parts.push(`${r.unpushed} unpushed ${r.unpushed === 1 ? "commit" : "commits"}`);
  else parts.push("pushed");
  if (r.parked) parts.push("parked changes");
  return parts.join(" · ");
}

export function FinishFeatureDialog({ env, feature, onClose, onDone }: Props) {
  const [rows, setRows] = useState<FinishRepo[] | null>(null);
  const [discard, setDiscard] = useState<Set<string>>(new Set());
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let live = true;
    api
      .sfFinishPreview(env.id, feature.id)
      .then((r) => live && setRows(r))
      .catch((e) => live && setError(errorText(e)));
    return () => {
      live = false;
    };
  }, [env.id, feature.id]);

  const atRisk = rows?.filter((r) => r.atRisk) ?? [];
  const unticked = atRisk.filter((r) => !discard.has(r.repoId));
  const active = env.activeFeatureId === feature.id;

  const toggle = (id: string) =>
    setDiscard((d) => {
      const n = new Set(d);
      if (n.has(id)) n.delete(id);
      else n.add(id);
      return n;
    });

  const finish = async () => {
    setBusy(true);
    setError(null);
    try {
      onDone(await api.sfFinishFeature(env.id, feature.id, [...discard]));
    } catch (e) {
      setError(errorText(e));
      setBusy(false);
    }
  };

  return (
    <div className="modal-backdrop" onMouseDown={() => !busy && onClose()}>
      <div className="modal sf-switch-modal" onMouseDown={(e) => e.stopPropagation()}>
        <div className="modal-title">Finish “{feature.name}”</div>
        <p>
          Deletes the local <span className="mono">{feature.branch}</span> branches and any parked changes.
          {active && " The repositories go back to base first."} Branches on origin are never deleted.
        </p>
        {!rows && !error && <div className="dim">Checking…</div>}
        {rows && rows.length === 0 && <p className="dim">No repository has this branch. Finishing just forgets it.</p>}
        {rows && rows.length > 0 && (
          <div className="sf-switch-steps">
            {rows.map((r) => (
              <label key={r.repoId} className={`sf-switch-row${r.atRisk ? " risk" : ""}`}>
                <span className="sf-switch-state">
                  {r.atRisk && (
                    <input type="checkbox" checked={discard.has(r.repoId)} onChange={() => toggle(r.repoId)} />
                  )}
                </span>
                <span className="sf-repo-name">{r.label}</span>
                <span className="dim small">{state(r)}</span>
                {r.atRisk && <span className="sf-risk small">{discard.has(r.repoId) ? "will be lost" : "kept"}</span>}
              </label>
            ))}
          </div>
        )}
        {unticked.length > 0 && (
          <p className="sf-warn-text small">
            Push or commit the work in {unticked.map((r) => r.label).join(", ")} first, or tick it to discard it.
          </p>
        )}
        {error && <pre className="commit-error">{error}</pre>}
        <div className="popover-actions">
          <button className="btn" onClick={onClose} disabled={busy}>
            Cancel
          </button>
          <button
            className={`btn ${discard.size ? "danger" : "primary"}`}
            onClick={() => void finish()}
            disabled={!rows || busy || unticked.length > 0}
          >
            {busy ? "Finishing…" : discard.size ? "Discard and finish" : "Finish feature"}
          </button>
        </div>
      </div>
    </div>
  );
}
