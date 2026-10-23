// Line-based three-way merge (diff3) for the conflict editor. Changes made on
// only one side merge cleanly; overlapping (or touching) changes on both sides
// become conflicts. It splits git's conflict hunks into smaller ones, so a file
// with two git hunks can show four conflicts.

export type Chunk =
  | { kind: "same"; lines: string[] }
  | {
      kind: "conflict";
      base: string[];
      ours: string[];
      theirs: string[];
      /** Where the conflict starts in the ours / theirs files (0-based line). */
      oursStart: number;
      theirsStart: number;
    };

export type Conflict = Extract<Chunk, { kind: "conflict" }>;

/** Lines of a text file; a trailing newline doesn't add an empty last line. */
export function splitLines(text: string): string[] {
  if (text === "") return [];
  const lines = text.split("\n");
  if (lines[lines.length - 1] === "") lines.pop();
  return lines;
}

/** Myers diff of a[a0..a1) and b[b0..b1): the matching [aIndex, bIndex] pairs. */
function myers(a: string[], b: string[], a0: number, a1: number, b0: number, b1: number): [number, number][] {
  const n = a1 - a0;
  const m = b1 - b0;
  if (n === 0 || m === 0) return [];
  const max = n + m;
  const off = max + 1;
  const v = new Int32Array(2 * max + 3);
  // trace[d] holds v (for k in -d-1..d+1) as it was before step d.
  const trace: Int32Array[] = [];
  for (let d = 0; d <= max; d++) {
    trace.push(v.slice(off - d - 1, off + d + 2));
    for (let k = -d; k <= d; k += 2) {
      let x = k === -d || (k !== d && v[off + k - 1] < v[off + k + 1]) ? v[off + k + 1] : v[off + k - 1] + 1;
      let y = x - k;
      while (x < n && y < m && a[a0 + x] === b[b0 + y]) {
        x++;
        y++;
      }
      v[off + k] = x;
      if (x >= n && y >= m) return backtrack(trace, n, m, a0, b0);
    }
  }
  return [];
}

function backtrack(trace: Int32Array[], n: number, m: number, a0: number, b0: number): [number, number][] {
  const pairs: [number, number][] = [];
  let x = n;
  let y = m;
  for (let d = trace.length - 1; d >= 0; d--) {
    const t = trace[d];
    const at = (k: number) => t[k + d + 1];
    const k = x - y;
    const prevK = k === -d || (k !== d && at(k - 1) < at(k + 1)) ? k + 1 : k - 1;
    const prevX = at(prevK);
    const prevY = prevX - prevK;
    while (x > prevX && y > prevY) {
      x--;
      y--;
      pairs.push([a0 + x, b0 + y]);
    }
    if (d > 0) {
      x = prevX;
      y = prevY;
    }
  }
  return pairs.reverse();
}

/** Matching line pairs between a and b (common prefix and suffix first, Myers in between). */
export function matchLines(a: string[], b: string[]): [number, number][] {
  let start = 0;
  while (start < a.length && start < b.length && a[start] === b[start]) start++;
  let endA = a.length;
  let endB = b.length;
  while (endA > start && endB > start && a[endA - 1] === b[endB - 1]) {
    endA--;
    endB--;
  }
  const pairs: [number, number][] = [];
  for (let i = 0; i < start; i++) pairs.push([i, i]);
  pairs.push(...myers(a, b, start, endA, start, endB));
  for (let i = 0; endA + i < a.length; i++) pairs.push([endA + i, endB + i]);
  return pairs;
}

/** A changed range: base [bs, be) became side [ss, se). */
interface Hunk {
  side: 0 | 1;
  bs: number;
  be: number;
  ss: number;
  se: number;
}

function hunks(base: string[], side: string[], which: 0 | 1): Hunk[] {
  const out: Hunk[] = [];
  let pb = 0;
  let ps = 0;
  for (const [i, j] of [...matchLines(base, side), [base.length, side.length]]) {
    if (i > pb || j > ps) out.push({ side: which, bs: pb, be: i, ss: ps, se: j });
    pb = i + 1;
    ps = j + 1;
  }
  return out;
}

const sameLines = (a: string[], b: string[]) => a.length === b.length && a.every((l, i) => l === b[i]);

export function merge3(base: string[], ours: string[], theirs: string[]): Chunk[] {
  const all = [...hunks(base, ours, 0), ...hunks(base, theirs, 1)].sort((x, y) => x.bs - y.bs || x.be - y.be);
  const sides = [ours, theirs];
  // How far each side's line numbers have drifted from base's, at the current position.
  const delta = [0, 0];
  const chunks: Chunk[] = [];
  const pushSame = (lines: string[]) => {
    if (lines.length === 0) return;
    const last = chunks[chunks.length - 1];
    if (last?.kind === "same") last.lines.push(...lines);
    else chunks.push({ kind: "same", lines: [...lines] });
  };

  let pos = 0;
  let i = 0;
  while (i < all.length) {
    const region = [all[i]];
    const bs = all[i].bs;
    let be = all[i].be;
    i++;
    while (i < all.length && all[i].bs <= be) {
      be = Math.max(be, all[i].be);
      region.push(all[i]);
      i++;
    }
    pushSame(base.slice(pos, bs));
    pos = be;

    const range = (s: 0 | 1): [number, number] => {
      const hs = region.filter((h) => h.side === s);
      if (hs.length === 0) return [bs + delta[s], be + delta[s]];
      const first = hs[0];
      const last = hs[hs.length - 1];
      return [first.ss - (first.bs - bs), last.se + (be - last.be)];
    };
    const [os, oe] = range(0);
    const [ts, te] = range(1);
    delta[0] = oe - be;
    delta[1] = te - be;
    const o = sides[0].slice(os, oe);
    const t = sides[1].slice(ts, te);
    const touched = new Set(region.map((h) => h.side));
    if (touched.size === 1) pushSame(touched.has(0) ? o : t);
    else if (sameLines(o, t)) pushSame(o);
    else chunks.push({ kind: "conflict", base: base.slice(bs, be), ours: o, theirs: t, oursStart: os, theirsStart: ts });
  }
  pushSame(base.slice(pos));
  return chunks;
}
