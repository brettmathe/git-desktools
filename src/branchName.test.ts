// Compares BRANCH_NAME_RE with the installed git. Run: pnpm test
import { execFileSync } from "node:child_process";
import assert from "node:assert/strict";
import { test } from "node:test";
import { BRANCH_NAME_RE, branchNameError } from "./branchName.ts";

function gitAccepts(name: string): boolean {
  try {
    execFileSync("git", ["check-ref-format", "--branch", name], { stdio: "ignore", cwd: "/" });
    return true;
  } catch {
    return false;
  }
}

// prettier-ignore
const CASES = [
  "feature/matchmaking-v2", "fix/DEX-1234_crash", "bob/wip", "a", "release/2.4", "émoji-ü", "x.y",
  "", "-x", "HEAD", "/a", "a/", "a//b", "a..b", "a@{b", "a.", "a/.b", ".a", "a.lock", "a.lock/b",
  "a/b.lock", "a b", "a~b", "a^b", "a:b", "a?b", "a*b", "a[b", "a\\b", "a\tb", "a\x7fb", "a@b", "@a",
  "a/HEAD", "HEADx", "a.locks", "a]b", "a{b", "lock", "a/.lock", "x/..y", "..", "a/.", "refs/heads/x",
];

test("matches git on hand-picked names", () => {
  for (const name of CASES) assert.equal(BRANCH_NAME_RE.test(name), gitAccepts(name), JSON.stringify(name));
});

test("matches git on random names", () => {
  const parts = ["a", "b", "/", ".", "-", "@", "{", "lock", ".lock", "H", "EAD", " ", "~", "^", ":", "?", "*", "[", "\\", "é", "_"];
  for (let i = 0; i < 500; i++) {
    let name = "";
    for (let j = 1 + Math.floor(Math.random() * 6); j > 0; j--) name += parts[Math.floor(Math.random() * parts.length)];
    // The one deliberate difference: git accepts a bare @ (it means the current branch).
    if (name === "@") continue;
    assert.equal(BRANCH_NAME_RE.test(name), gitAccepts(name), JSON.stringify(name));
  }
});

test("a bare @ is rejected", () => assert.equal(BRANCH_NAME_RE.test("@"), false));

test("messages name the broken rule", () => {
  assert.equal(branchNameError("feature/x"), null);
  assert.match(branchNameError("has space")!, /spaces/);
  assert.match(branchNameError("x.lock")!, /\.lock/);
  assert.match(branchNameError("a..b")!, /\.\./);
  assert.match(branchNameError("-x")!, /start with -/);
  for (const name of CASES) {
    if (!BRANCH_NAME_RE.test(name)) assert.notEqual(branchNameError(name), "Not a valid branch name", JSON.stringify(name));
  }
});
