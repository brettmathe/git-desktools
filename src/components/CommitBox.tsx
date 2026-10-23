import type { KeyboardEvent, ReactNode } from "react";

export interface CommitDraft {
  summary: string;
  description: string;
  amend: boolean;
  saved: { summary: string; description: string } | null;
}

export const emptyDraft: CommitDraft = { summary: "", description: "", amend: false, saved: null };
const SUMMARY_LIMIT = 72;

/** "summary\n\ndescription\n" */
export function draftMessage(d: CommitDraft): string {
  return d.description.trim() ? `${d.summary.trim()}\n\n${d.description.trim()}\n` : `${d.summary.trim()}\n`;
}

/** "Merge branch 'x'\n\nbody" → a draft. */
export function messageDraft(message: string): CommitDraft {
  const [summary, ...rest] = message.split("\n");
  return { ...emptyDraft, summary: summary.trim(), description: rest.join("\n").trim() };
}

interface Props {
  draft: CommitDraft;
  setDraft: (fn: (d: CommitDraft) => CommitDraft) => void;
  busy: boolean;
  /** Shows the Amend checkbox. */
  onAmend?: (amend: boolean) => void;
  readOnly?: boolean;
  error: string | null;
  /** Cmd/Ctrl+Enter. */
  onSubmit: () => void;
  /** Buttons (and anything else) under the message. */
  children: ReactNode;
}

/** Message inputs pinned at the bottom of the WIP and conflict panels. */
export function CommitBox({ draft, setDraft, busy, onAmend, readOnly, error, onSubmit, children }: Props) {
  const onKey = (e: KeyboardEvent) => {
    if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
      e.preventDefault();
      onSubmit();
    }
  };
  const remaining = SUMMARY_LIMIT - draft.summary.length;

  return (
    <div className="commit-box" onKeyDown={onKey}>
      {onAmend && (
        <label className="amend">
          <input type="checkbox" checked={draft.amend} disabled={busy} onChange={(e) => onAmend(e.target.checked)} />
          Amend previous commit
        </label>
      )}
      <div className="commit-inputs">
        <div className="summary-row">
          <input
            className="summary-input"
            placeholder="Commit summary"
            value={draft.summary}
            readOnly={readOnly}
            onChange={(e) => setDraft((d) => ({ ...d, summary: e.target.value }))}
            spellCheck
          />
          <span className={`counter${remaining < 0 ? " over" : remaining < 10 ? " near" : ""}`}>{remaining}</span>
        </div>
        <textarea
          className="desc-input"
          placeholder="Description"
          value={draft.description}
          readOnly={readOnly}
          onChange={(e) => setDraft((d) => ({ ...d, description: e.target.value }))}
        />
      </div>
      {error && <pre className="commit-error">{error}</pre>}
      {children}
    </div>
  );
}
