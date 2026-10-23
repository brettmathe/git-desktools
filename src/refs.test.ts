// Run: pnpm test
import assert from "node:assert/strict";
import { test } from "node:test";
import { checkoutTarget } from "./refs.ts";
import type { RefLabel } from "./types.ts";

const local = (name: string, isHead = false): RefLabel => ({ name, kind: "local", remote: null, isHead });
const remote = (name: string, r = "origin"): RefLabel => ({ name, kind: "remote", remote: r, isHead: false });
const tag = (name: string): RefLabel => ({ name, kind: "tag", remote: null, isHead: false });

test("prefers a local branch that isn't checked out", () => {
  assert.deepEqual(checkoutTarget([local("develop"), remote("develop")]), { branch: "develop", track: null });
  assert.deepEqual(checkoutTarget([local("main", true), local("develop")]), { branch: "develop", track: null });
});

test("falls back to a remote-only branch, tracking it", () => {
  assert.deepEqual(checkoutTarget([local("main", true), remote("main"), remote("feat/x")]), {
    branch: "feat/x",
    track: "origin/feat/x",
  });
});

test("nothing to do on the checked-out branch, tags or origin/HEAD", () => {
  assert.equal(checkoutTarget([local("main", true), remote("main")]), null);
  assert.equal(checkoutTarget([tag("v1.0")]), null);
  assert.equal(checkoutTarget([remote("HEAD")]), null);
  assert.equal(checkoutTarget([]), null);
});
