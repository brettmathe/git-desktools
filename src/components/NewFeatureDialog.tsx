import { useEffect, useState } from "react";
import { api, errorText } from "../api";
import { branchNameError } from "../branchName";
import type { AutoBranched, BranchCheck, SmartEnv } from "../types";

interface Props {
  env: SmartEnv;
  onClose: () => void;
  onStarted: (result: { env: SmartEnv; moved: AutoBranched[]; warnings: string[] }, pullBases: boolean) => void;
}

const CHECK_DELAY_MS = 250;

export function NewFeatureDialog({ env, onClose, onStarted }: Props) {
  const [branch, setBranch] = useState("");
  const [name, setName] = useState("");
  const [pullBases, setPullBases] = useState(false);
  const [check, setCheck] = useState<{ branch: string; result: BranchCheck } | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const typed = branch.trim();
  const localError = typed ? branchNameError(typed) : null;
  const active = env.features.find((f) => f.id === env.activeFeatureId) ?? null;

  // The regex is instant; the backend adds git's own check, base/feature clashes,
  // a vs a/b conflicts, and where the branch already exists.
  useEffect(() => {
    if (!typed || localError) return;
    let live = true;
    const t = window.setTimeout(() => {
      api
        .sfCheckBranch(env.id, typed)
        .then((result) => live && setCheck({ branch: typed, result }))
        .catch((e) => live && setError(errorText(e)));
    }, CHECK_DELAY_MS);
    return () => {
      live = false;
      window.clearTimeout(t);
    };
  }, [env.id, typed, localError]);

  const current = check?.branch === typed ? check.result : null;
  const problem = localError ?? current?.error ?? null;
  const existing = current?.repos.filter((r) => r.local) ?? [];
  const onRemote = current?.repos.filter((r) => r.remote && !r.local) ?? [];
  const moving = current?.repos.filter((r) => r.dirtyOnBase && !r.local) ?? [];
  const adopt = existing.length > 0;
  const ready = !!typed && !problem && !!current && !busy;

  const start = async () => {
    if (!ready) return;
    setBusy(true);
    setError(null);
    try {
      onStarted(await api.sfStartFeature(env.id, name, typed, adopt), pullBases);
    } catch (e) {
      setError(errorText(e));
      setBusy(false);
    }
  };

  const labels = (rs: { label: string }[]) => rs.map((r) => r.label).join(", ");

  return (
    <div className="modal-backdrop" onMouseDown={() => !busy && onClose()}>
      <div className="modal sf-feature-modal" onMouseDown={(e) => e.stopPropagation()}>
        <div className="modal-title">New feature in {env.name}</div>
        <label className="field">
          <span>Branch name</span>
          <input
            autoFocus
            className="mono"
            value={branch}
            placeholder="e.g. feature/matchmaking-v2"
            spellCheck={false}
            autoCapitalize="off"
            autoCorrect="off"
            onChange={(e) => setBranch(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && void start()}
          />
        </label>
        {typed && problem && <div className="sf-field-error">{problem}</div>}
        <label className="field">
          <span>Display name (optional)</span>
          <input
            value={name}
            placeholder={typed || "Defaults to the branch name"}
            onChange={(e) => setName(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && void start()}
          />
        </label>

        <div className="sf-feature-notes">
          <p className="dim">
            No branches are created yet. A repo moves onto <span className="mono">{typed || "the branch"}</span> as soon
            as one of its files changes.
          </p>
          {!problem && adopt && (
            <p>
              <span className="mono">{typed}</span> already exists in {labels(existing)}. Starting adopts it there and
              checks it out.
            </p>
          )}
          {!problem && moving.length > 0 && (
            <p>
              {labels(moving)} {moving.length === 1 ? "has" : "have"} uncommitted changes on base and will move to the
              new branch right away.
            </p>
          )}
          {!problem && onRemote.length > 0 && (
            <p className="dim">
              origin/{typed} exists in {labels(onRemote)}. It isn't checked out automatically; use <i>Check out</i> in
              the panel to work on it.
            </p>
          )}
          {active && (
            <p className="sf-warn-text">
              “{active.name}” is active. Starting this feature sends its repos back to base, so commit its changes
              first.
            </p>
          )}
        </div>

        <label className="check-row">
          <input type="checkbox" checked={pullBases} onChange={(e) => setPullBases(e.target.checked)} />
          Pull base branches first (fast-forward only)
        </label>

        {error && <pre className="commit-error">{error}</pre>}
        <div className="popover-actions">
          <button className="btn" onClick={onClose} disabled={busy}>
            Cancel
          </button>
          <button className="btn primary" onClick={() => void start()} disabled={!ready}>
            {busy ? "Starting…" : adopt ? "Adopt and start" : "Start feature"}
          </button>
        </div>
      </div>
    </div>
  );
}
