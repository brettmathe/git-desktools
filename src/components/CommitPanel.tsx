import { useEffect, useState } from "react";
import { api, errorText } from "../api";
import type { CommitDetail, FileChange, FileViewMode } from "../types";
import { fileStats, formatDate, shortSha } from "../util";
import { showToast } from "../toast";
import { Avatar } from "./Avatar";
import { FileList, ViewModeToggle } from "./FileList";
import { StatsLine } from "./StatsLine";

interface Props {
  path: string;
  commitId: string;
  viewMode: FileViewMode;
  onViewMode: (m: FileViewMode) => void;
  openFilePath: string | null;
  onOpenFile: (file: FileChange) => void;
  onSelectCommit: (id: string) => void;
}

export function CommitPanel({ path, commitId, viewMode, onViewMode, openFilePath, onOpenFile, onSelectCommit }: Props) {
  const [detail, setDetail] = useState<CommitDetail | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let live = true;
    setError(null);
    api
      .commitDetail(path, commitId)
      .then((d) => live && setDetail(d))
      .catch((e) => live && setError(errorText(e)));
    return () => {
      live = false;
    };
  }, [path, commitId]);

  if (error) return <div className="detail"><div className="panel-error">{error}</div></div>;
  if (!detail || detail.id !== commitId) return <div className="detail"><div className="panel-loading">Loading…</div></div>;

  const { author, committer } = detail;
  const committerDiffers = committer.email !== author.email || committer.name !== author.name;

  return (
    <div className="detail">
      <div className="detail-head">
        <span className="dim">commit:</span>
        <button
          className="sha-btn"
          title="Copy full SHA"
          onClick={() => navigator.clipboard.writeText(detail.id).then(() => showToast("Copied commit SHA", "success"))}
        >
          {shortSha(detail.id)}
        </button>
      </div>
      <div className="msg-box">
        <div className="msg-summary">{detail.summary}</div>
        {detail.body && <div className="msg-body">{detail.body}</div>}
      </div>
      <div className="people">
        <div className="person">
          <Avatar {...author} size={38} />
          <div className="person-text">
            <div className="person-name">{author.name}</div>
            <div className="person-meta">authored {formatDate(author.time)}</div>
          </div>
          {detail.parents.length > 0 && (
            <div className="parents">
              <span className="dim">parent{detail.parents.length > 1 ? "s" : ""}:</span>
              {detail.parents.map((p) => (
                <button key={p} className="link-btn mono" onClick={() => onSelectCommit(p)} title={p}>
                  {shortSha(p)}
                </button>
              ))}
            </div>
          )}
        </div>
        {committerDiffers && (
          <div className="person">
            <Avatar {...committer} size={38} />
            <div className="person-text">
              <div className="person-name">{committer.name}</div>
              <div className="person-meta">committed {formatDate(committer.time)}</div>
            </div>
          </div>
        )}
        {detail.coAuthors.length > 0 && (
          <div className="coauthors">
            <span className="dim">Co-authors:</span>
            {detail.coAuthors.map((c) => (
              <Avatar key={c.email + c.name} {...c} size={24} title={`${c.name} <${c.email}>`} />
            ))}
          </div>
        )}
      </div>
      <div className="files-bar">
        <StatsLine stats={fileStats(detail.files)} />
        <ViewModeToggle mode={viewMode} onChange={onViewMode} />
      </div>
      <div className="files-scroll">
        <FileList files={detail.files} mode={viewMode} selectedPath={openFilePath} onOpen={onOpenFile} showStats />
        {detail.filesTruncated && <div className="dim small pad">Only the first 5,000 files are listed.</div>}
        {detail.files.length === 0 && <div className="dim small pad">No file changes.</div>}
      </div>
    </div>
  );
}
