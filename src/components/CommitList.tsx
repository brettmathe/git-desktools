import { memo, useEffect, useLayoutEffect, useMemo, useRef, useState, type KeyboardEvent, type WheelEvent } from "react";
import { WIP_COLOR, WIP_ID, type Author, type CommitRow, type RefLabel, type RepoData, type Seg } from "../types";
import { checkoutTarget, toPills } from "../refs";
import { dateBucket, laneColor, WIP_STROKE, type FileStats } from "../util";
import { Avatar } from "./Avatar";
import { CheckIcon, CloudIcon, GitHubIcon, LaptopIcon, PencilIcon, PlusIcon, TagIcon, WarningIcon, XIcon } from "./icons";
import { clamp, Resizer } from "./Resizer";

export const ROW_H = 30;
const LANE_W = 22;
const NODE_R = 11;
const OVERSCAN = 12;
const GRAPH_PAD = 6;

const laneX = (lane: number) => GRAPH_PAD + lane * LANE_W + LANE_W / 2;
const rowY = (y: number) => (y === 0 ? 0 : y === 1 ? ROW_H / 2 : ROW_H);

function segPath([l1, y1, l2, y2]: Seg): string {
  const x1 = laneX(l1);
  const x2 = laneX(l2);
  const a = rowY(y1);
  const b = rowY(y2);
  if (x1 === x2) return `M${x1} ${a}L${x2} ${b}`;
  // Converging into a node: drop down, then bend in. Leaving a node: bend out, then drop.
  return y1 === 0 ? `M${x1} ${a}Q${x1} ${b} ${x2} ${b}` : `M${x1} ${a}Q${x2} ${a} ${x2} ${b}`;
}

export interface ListHandle {
  scrollToId: (id: string, center?: boolean) => void;
  focus: () => void;
}

interface Props {
  data: RepoData;
  wipStats: FileStats | null;
  /** A merge, rebase, etc. is in progress: shown on the WIP row instead of the counts. */
  opNote: string | null;
  selected: string | null;
  onSelect: (id: string) => void;
  /** Double-click a row: check out a branch that points at it. */
  onCheckout: (branch: string, track: string | null) => void;
  handleRef: React.RefObject<ListHandle | null>;
}

export function CommitList({ data, wipStats, opNote, selected, onSelect, onCheckout, handleRef }: Props) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const [scrollTop, setScrollTop] = useState(0);
  const [viewH, setViewH] = useState(800);
  const [refW, setRefW] = useState(170);
  const graphContentW = data.maxLanes * LANE_W + GRAPH_PAD * 2;
  const [graphW, setGraphW] = useState(() => clamp(graphContentW, 70, 280));
  const [graphX, setGraphX] = useState(0);
  const maxGraphX = Math.max(0, graphContentW - graphW);

  useEffect(() => {
    setGraphX((x) => clamp(x, 0, maxGraphX));
  }, [maxGraphX]);

  useLayoutEffect(() => {
    const el = scrollRef.current!;
    const ro = new ResizeObserver(() => setViewH(el.clientHeight || 800));
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  const commits = data.commits;
  const indexOf = useMemo(() => new Map(commits.map((c, i) => [c.id, i])), [commits]);
  const buckets = useMemo(() => {
    const now = Date.now() / 1000;
    let prev = "";
    return commits.map((c) => {
      if (c.id === WIP_ID) return null;
      const b = dateBucket(c.time, now);
      const label = b !== prev ? b : null;
      prev = b;
      return label;
    });
  }, [commits]);
  const githubRemotes = useMemo(
    () => new Set(data.remoteInfos.filter((r) => r.url?.includes("github.com")).map((r) => r.name)),
    [data.remoteInfos],
  );

  const scrollToIndex = (i: number, center: boolean) => {
    const el = scrollRef.current;
    if (!el) return;
    const top = i * ROW_H;
    if (center) {
      el.scrollTop = Math.max(0, top - el.clientHeight / 2 + ROW_H / 2);
    } else if (top < el.scrollTop) {
      el.scrollTop = top;
    } else if (top + ROW_H > el.scrollTop + el.clientHeight) {
      el.scrollTop = top + ROW_H - el.clientHeight;
    }
  };

  useEffect(() => {
    handleRef.current = {
      scrollToId: (id, center = true) => {
        const i = indexOf.get(id);
        if (i !== undefined) scrollToIndex(i, center);
      },
      focus: () => scrollRef.current?.focus(),
    };
  });

  const onKeyDown = (e: KeyboardEvent) => {
    if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
    e.preventDefault();
    const cur = selected ? (indexOf.get(selected) ?? -1) : -1;
    const next = clamp(cur + (e.key === "ArrowDown" ? 1 : -1), 0, commits.length - 1);
    if (commits[next]) {
      onSelect(commits[next].id);
      scrollToIndex(next, false);
    }
  };

  const onWheel = (e: WheelEvent) => {
    if (maxGraphX > 0 && Math.abs(e.deltaX) > Math.abs(e.deltaY)) {
      setGraphX((x) => clamp(x + e.deltaX, 0, maxGraphX));
    }
  };

  const first = Math.max(0, Math.floor(scrollTop / ROW_H) - OVERSCAN);
  const last = Math.min(commits.length, Math.ceil((scrollTop + viewH) / ROW_H) + OVERSCAN);
  const rows = [];
  for (let i = first; i < last; i++) {
    const c = commits[i];
    rows.push(
      <Row
        key={c.id}
        c={c}
        index={i}
        selected={c.id === selected}
        author={data.authors[c.author]}
        refW={refW}
        graphW={graphW}
        graphX={graphX}
        graphContentW={graphContentW}
        bucket={buckets[i]}
        wipStats={c.id === WIP_ID ? wipStats : null}
        opNote={c.id === WIP_ID ? opNote : null}
        githubRemotes={githubRemotes}
        onSelect={onSelect}
        onCheckout={onCheckout}
      />,
    );
  }

  return (
    <div className="commit-list">
      <div className="cl-header">
        <div className="cl-h" style={{ width: refW }}>
          Branch / Tag
          <Resizer onDrag={(d) => setRefW((w) => clamp(w + d, 60, 420))} />
        </div>
        <div className="cl-h" style={{ width: graphW }}>
          Graph
          {maxGraphX > 0 && (
            <input
              className="graph-scroll"
              type="range"
              min={0}
              max={maxGraphX}
              value={graphX}
              onChange={(e) => setGraphX(Number(e.target.value))}
              title="Scroll the graph horizontally (or use a horizontal scroll gesture)"
            />
          )}
          <Resizer onDrag={(d) => setGraphW((w) => clamp(w + d, 40, Math.max(80, graphContentW + 40)))} />
        </div>
        <div className="cl-h cl-h-msg">Commit Message</div>
      </div>
      <div
        className="cl-scroll"
        ref={scrollRef}
        tabIndex={0}
        onScroll={(e) => setScrollTop(e.currentTarget.scrollTop)}
        onKeyDown={onKeyDown}
        onWheel={onWheel}
      >
        <div className="cl-canvas" style={{ height: commits.length * ROW_H + (data.truncated ? ROW_H * 2 : 0) }}>
          {rows}
          {data.truncated && (
            <div className="cl-truncated" style={{ top: commits.length * ROW_H }}>
              Showing the newest {commits.length.toLocaleString()} commits.
            </div>
          )}
        </div>
        {commits.length === 0 && <div className="empty-note">No commits yet.</div>}
      </div>
    </div>
  );
}

interface RowProps {
  c: CommitRow;
  index: number;
  selected: boolean;
  author: Author | undefined;
  refW: number;
  graphW: number;
  graphX: number;
  graphContentW: number;
  bucket: string | null;
  wipStats: FileStats | null;
  opNote: string | null;
  githubRemotes: Set<string>;
  onSelect: (id: string) => void;
  onCheckout: (branch: string, track: string | null) => void;
}

const Row = memo(function Row(p: RowProps) {
  const { c } = p;
  const isWip = c.id === WIP_ID;
  const color = laneColor(c.color);
  const nx = laneX(c.lane);
  const hasRefs = c.refs.length > 0;
  const tooltip = isWip ? (p.opNote ?? "Uncommitted changes") : c.body ? `${c.summary}\n\n${c.body}` : c.summary;

  return (
    <div
      className={`cl-row${p.selected ? " selected" : ""}${isWip ? " wip" : ""}${p.bucket && p.index > 0 ? " bucket-start" : ""}`}
      style={{ top: p.index * ROW_H }}
      onClick={() => p.onSelect(c.id)}
      onDoubleClick={() => {
        const t = checkoutTarget(c.refs);
        if (t) p.onCheckout(t.branch, t.track);
      }}
    >
      <div className="cl-refs" style={{ width: p.refW }}>
        {hasRefs && <RefPills refs={c.refs} color={color} githubRemotes={p.githubRemotes} />}
      </div>
      <div className="cl-graph" style={{ width: p.graphW }}>
        <div className="cl-graph-inner" style={{ width: p.graphContentW, transform: `translateX(${-p.graphX}px)` }}>
          <svg width={p.graphContentW} height={ROW_H} className="cl-svg">
            {!isWip && !p.selected && (
              <rect x={nx} y={3} width={Math.max(0, p.graphContentW - nx)} height={ROW_H - 6} fill={color} opacity={0.09} />
            )}
            {hasRefs && <line x1={p.graphX} y1={ROW_H / 2} x2={nx} y2={ROW_H / 2} stroke={color} strokeWidth={1.5} />}
            {c.segs.map((s, i) => (
              <path
                key={i}
                d={segPath(s)}
                stroke={laneColor(s[4])}
                strokeWidth={2}
                fill="none"
                strokeDasharray={s[4] === WIP_COLOR ? "3 3" : undefined}
              />
            ))}
            {isWip ? (
              <circle cx={nx} cy={ROW_H / 2} r={NODE_R - 2} fill="#1e2128" stroke={WIP_STROKE} strokeWidth={2} strokeDasharray="3.2 2.4" />
            ) : c.merge ? (
              <circle cx={nx} cy={ROW_H / 2} r={5} fill={color} />
            ) : (
              <circle cx={nx} cy={ROW_H / 2} r={NODE_R} fill={color} />
            )}
          </svg>
          {!isWip && !c.merge && p.author && (
            <div className="cl-node" style={{ left: nx - NODE_R + 2, top: ROW_H / 2 - NODE_R + 2 }}>
              <Avatar {...p.author} size={NODE_R * 2 - 4} />
            </div>
          )}
        </div>
      </div>
      <div className="cl-msg" style={{ borderLeftColor: isWip ? WIP_STROKE : color }} title={tooltip}>
        {isWip && p.opNote ? (
          <span className="cl-op-note">
            <WarningIcon size={14} /> {p.opNote}
          </span>
        ) : isWip ? (
          <>
            <span className="cl-wip-label">// WIP</span>
            {p.wipStats && <WipCounts stats={p.wipStats} />}
          </>
        ) : (
          <>
            <span className="cl-summary">{c.summary}</span>
            {c.body && <span className="cl-body">{c.body.replace(/\s*\n+\s*/g, " | ")}</span>}
          </>
        )}
      </div>
      {p.bucket && p.index > 0 && <span className="cl-date">{p.bucket}</span>}
    </div>
  );
});

export function WipCounts({ stats }: { stats: FileStats }) {
  return (
    <span className="wip-counts">
      {stats.modified + stats.renamed > 0 && (
        <span className="st-modified">
          <PencilIcon size={12} /> {stats.modified + stats.renamed}
        </span>
      )}
      {stats.added > 0 && (
        <span className="st-added">
          <PlusIcon size={12} /> {stats.added}
        </span>
      )}
      {stats.deleted > 0 && (
        <span className="st-deleted">
          <XIcon size={12} /> {stats.deleted}
        </span>
      )}
    </span>
  );
}

function refTitle(r: RefLabel): string {
  if (r.kind === "remote") return `${r.remote}/${r.name} (remote)`;
  if (r.kind === "tag") return `${r.name} (tag)`;
  return `${r.name} (local${r.isHead ? ", checked out" : ""})`;
}

function RefPills({ refs, color, githubRemotes }: { refs: RefLabel[]; color: string; githubRemotes: Set<string> }) {
  const pills = toPills(refs);
  const pill = pills[0];
  const head = pill.local?.isHead ?? false;
  return (
    <div className="ref-pills" title={refs.map(refTitle).join("\n")}>
      <span
        className={`ref-pill${head ? " head" : ""}${!pill.local && !pill.tag ? " remote-only" : ""}`}
        style={{ borderColor: color, background: `color-mix(in srgb, ${color} ${head ? 55 : 32}%, #1e2128)` }}
      >
        {head && <CheckIcon size={12} />}
        <span className="ref-name">{pill.name}</span>
        {pill.local && <LaptopIcon size={12} />}
        {pill.remotes.length > 0 &&
          (pill.remotes.some((r) => githubRemotes.has(r.remote ?? "")) ? <GitHubIcon size={12} /> : <CloudIcon size={12} />)}
        {pill.tag && <TagIcon size={12} />}
      </span>
      {pills.length > 1 && <span className="ref-more">+{pills.length - 1}</span>}
      <span className="ref-line" style={{ background: color }} />
    </div>
  );
}
