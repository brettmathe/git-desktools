// Run: pnpm test
import assert from "node:assert/strict";
import { test } from "node:test";
import { matchLines, merge3, splitLines, type Conflict } from "./merge3.ts";

const conflicts = (chunks: ReturnType<typeof merge3>) => chunks.filter((c): c is Conflict => c.kind === "conflict");
const L = (s: string) => splitLines(s);

test("splitLines ignores the trailing newline", () => {
  assert.deepEqual(splitLines(""), []);
  assert.deepEqual(splitLines("a\nb\n"), ["a", "b"]);
  assert.deepEqual(splitLines("a\nb"), ["a", "b"]);
  assert.deepEqual(splitLines("\n"), [""]);
});

test("matchLines finds a longest common subsequence", () => {
  const a = L("a\nb\nc\nd\ne\n");
  const b = L("a\nx\nc\ne\ny\n");
  assert.deepEqual(matchLines(a, b), [
    [0, 0],
    [2, 2],
    [4, 3],
  ]);
  assert.deepEqual(matchLines([], ["a"]), []);
});

test("one-sided changes merge cleanly", () => {
  const base = L("1\n2\n3\n4\n5\n");
  const ours = L("1\nTWO\n3\n4\n5\n");
  const theirs = L("1\n2\n3\n4\nFIVE\nsix\n");
  assert.deepEqual(merge3(base, ours, theirs), [{ kind: "same", lines: ["1", "TWO", "3", "4", "FIVE", "six"] }]);
});

test("identical changes on both sides are not a conflict", () => {
  const base = L("a\nb\nc\n");
  const both = L("a\nB\nc\n");
  assert.deepEqual(merge3(base, both, both), [{ kind: "same", lines: ["a", "B", "c"] }]);
});

test("both-added file with no base is one conflict", () => {
  const chunks = merge3([], L("# Notes (main)\n\n- shouted\n"), L("# Notes (feature)\n\n- templates\n"));
  assert.deepEqual(chunks, [
    {
      kind: "conflict",
      base: [],
      ours: ["# Notes (main)", "", "- shouted"],
      theirs: ["# Notes (feature)", "", "- templates"],
      oursStart: 0,
      theirsStart: 0,
    },
  ]);
});

test("deleted on one side, changed on the other", () => {
  const chunks = merge3(L("a\nb\nc\n"), L("a\nc\n"), L("a\nB\nc\n"));
  assert.deepEqual(conflicts(chunks), [
    { kind: "conflict", base: ["b"], ours: [], theirs: ["B"], oursStart: 1, theirsStart: 1 },
  ]);
});

// The fixture repo's app.js (git shows 2 conflict hunks; split finer, it is 4).
const BASE = `// Simple greeting app
const DEFAULT_NAME = "World";

function greet(name) {
  return "Hello, " + name + "!";
}

function farewell(name) {
  return "Goodbye, " + name + ".";
}

function add(a, b) {
  return a + b;
}

function multiply(a, b) {
  return a * b;
}

function main() {
  console.log(greet(DEFAULT_NAME));
  console.log(add(2, 3));
  console.log(farewell(DEFAULT_NAME));
}

main();
`;
const OURS = BASE.replace('"World"', '"Everyone"')
  .replace('"Hello, " + name + "!"', '"Hello, " + name.toUpperCase() + "!!"')
  .replace('"Goodbye, " + name + "."', '"Farewell, " + name + "!"')
  .replace("  console.log(add(2, 3));\n", "  console.log(add(2, 3));\n  console.log(multiply(4, 5));\n")
  .replace("\nmain();", "\nmain().catch(console.error);");
const THEIRS = BASE.replace('"World"', '"Friend"')
  .replace('"Hello, " + name + "!"', "`Hi there, ${name}!`")
  .replace('"Goodbye, " + name + "."', "`See you later, ${name}.`")
  .replace("  return a * b;", "  // keep it simple\n  return a * b;")
  .replace("\nmain();", "\nif (require.main === module) main();");

test("fixture app.js: four conflicts, clean changes from both sides kept", () => {
  const chunks = merge3(L(BASE), L(OURS), L(THEIRS));
  const cs = conflicts(chunks);
  assert.equal(cs.length, 4);
  assert.deepEqual(
    cs.map((c) => [c.base, c.ours, c.theirs, c.oursStart, c.theirsStart]),
    [
      [['const DEFAULT_NAME = "World";'], ['const DEFAULT_NAME = "Everyone";'], ['const DEFAULT_NAME = "Friend";'], 1, 1],
      [
        ['  return "Hello, " + name + "!";'],
        ['  return "Hello, " + name.toUpperCase() + "!!";'],
        ["  return `Hi there, ${name}!`;"],
        4,
        4,
      ],
      [
        ['  return "Goodbye, " + name + ".";'],
        ['  return "Farewell, " + name + "!";'],
        ["  return `See you later, ${name}.`;"],
        8,
        8,
      ],
      [["main();"], ["main().catch(console.error);"], ["if (require.main === module) main();"], 26, 26],
    ],
  );
  const merged = chunks.flatMap((c) => (c.kind === "same" ? c.lines : c.base));
  assert.ok(merged.includes("  // keep it simple"));
  assert.ok(merged.includes("  console.log(multiply(4, 5));"));
  assert.equal(merged.length, L(BASE).length + 2);
});
