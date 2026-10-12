import { useEffect, useState } from "react";
import { api, errorText, onSwitchProgress } from "../api";
import type { Preflight, SmartEnv, SmartFeature, SwitchResult, SwitchStep } from "../types";
import { CheckIcon, WarningIcon } from "./icons";

interface Props {
  env: SmartEnv;
  /** null = Back to base. */
  target: SmartFeature | null;
  onClose: () => void;
  /** Called after a run, successful or not (the environment changed either way). */
  onDone: (result: SwitchResult | null) => void;
}

const stepKey = (s: SwitchStep) => `${s.repoId}:${s.action}`;

function describe(s: SwitchStep, env: SmartEnv): string | null {
  const name = env.features.find((f) => f.id === s.featureId)?.name ?? "feature";
  switch (s.action) {
    case "park":
      return s.detail ? `park changes of “${name}”` : null;
    case "base":
    case "switch":
      return `→ ${s.detail}`;
    case "restore":
      return `restore parked changes of “${name}”`;
  }
}

export function SwitchFeatureDialog({ env, target, onClose, onDone }: Props) {
  const [pre, setPre] = useState<Preflight | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [running, setRunning] = useState(false);
  const [progress, setProgress] = useState<Record<string, SwitchStep["status"]>>({});

  useEffect(() => {
    let live = true;
    api
      .sfPreflight(env.id, target?.id ?? null)
      .then((p) => live && setPre(p))
      .catch((e) => live && setError(errorText(e)));
    return () => {
      live = false;
    };
  }, [env.id, target?.id]);

  useEffect(() => {
    if (!running) return;
    return onSwitchProgress(env.id, (s) => setProgress((p) => ({ ...p, [stepKey(s)]: s.status })));
  }, [running, env.id]);

  const run = async () => {
    setRunning(true);
    setError(null);
    try {
      const result = target ? await api.sfSwitchFeature(env.id, target.id) : await api.sfBackToBase(env.id);
      onDone(result);
    } catch (e) {
      setError(errorText(e));
      setRunning(false);
      onDone(null);
    }
  };

  const title = target ? `Switch to “${target.name}”` : `Back to base in ${env.name}`;
  const steps = pre?.steps ?? [];
  const parks = steps.filter((s) => s.action === "park" && s.detail).length;
  const switches = steps.filter((s) => s.action === "switch").length;
  const restores = steps.filter((s) => s.action === "restore").length;
  const bases = steps.filter((s) => s.action === "base").length;
  const byRepo = new Map<string, { label: string; steps: SwitchStep[] }>();
  for (const s of steps) {
    const row = byRepo.get(s.repoId) ?? { label: s.label, steps: [] };
    row.steps.push(s);
    byRepo.set(s.repoId, row);
  }
  const summary = [
    parks && `park changes in ${parks} ${parks === 1 ? "repo" : "repos"}`,
    bases && `return ${bases} to base`,
    switches && `check out ${target?.branch} in ${switches}`,
    restores && `restore ${restores} parked ${restores === 1 ? "change" : "changes"}`,
  ].filter(Boolean);
  const blocked = !!pre && pre.problems.length > 0;

  return (
    <div className="modal-backdrop" onMouseDown={() => !running && onClose()}>
      <div className="modal sf-switch-modal" onMouseDown={(e) => e.stopPropagation()}>
        <div className="modal-title">{title}</div>
        {!pre && !error && <div className="dim">Checking every repository…</div>}
        {blocked && (
          <>
            <p>These need attention before switching. Nothing was changed.</p>
            <ul className="sf-problems">
              {pre.problems.map((p) => (
                <li key={p}>
                  <WarningIcon size={12} /> {p}
                </li>
              ))}
            </ul>
          </>
        )}
        {pre && !blocked && (
          <>
            <p>
              {summary.length
                ? `${summary.join(", ").replace(/^./, (c) => c.toUpperCase())}.`
                : "Every repository is already where it should be."}
            </p>
            {byRepo.size > 0 && (
              <div className="sf-switch-steps">
                {[...byRepo.entries()].map(([id, row]) => {
                  const lines = row.steps
                    .map((s) => ({ s, text: describe(s, env) }))
                    .filter((x): x is { s: SwitchStep; text: string } => !!x.text);
                  if (!lines.length) return null;
                  const done = row.steps.every((s) => progress[stepKey(s)] === "done");
                  const warned = row.steps.some((s) => progress[stepKey(s)] === "warning");
                  return (
                    <div key={id} className="sf-switch-row">
                      <span className="sf-switch-state">
                        {done ? <CheckIcon size={12} /> : warned ? <WarningIcon size={12} className="sf-warn" /> : null}
                      </span>
                      <span className="sf-repo-name">{row.label}</span>
                      <span className="dim small">{lines.map((l) => l.text).join(" · ")}</span>
                    </div>
                  );
                })}
              </div>
            )}
            {steps.length > 0 && (
              <p className="dim small">
                Files in these repositories change on disk. Editors pick that up; running builds, dev servers or
                terminals in them may need a restart. Uncommitted work is parked under its feature, not lost.
              </p>
            )}
          </>
        )}
        {error && <pre className="commit-error">{error}</pre>}
        <div className="popover-actions">
          <button className="btn" onClick={onClose} disabled={running}>
            {error ? "Close" : "Cancel"}
          </button>
          {!error && (
            <button className="btn primary" onClick={() => void run()} disabled={!pre || blocked || running}>
              {running ? "Switching…" : target ? "Switch" : "Back to base"}
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
