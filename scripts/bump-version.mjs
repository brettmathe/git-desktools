#!/usr/bin/env node
// Set the app version everywhere it appears, or verify the sites agree.
//
//   node scripts/bump-version.mjs 0.2.0     bump every site
//   node scripts/bump-version.mjs --check   verify they already agree
//
// tauri.conf.json is authoritative: getVersion() reports it, the updater compares
// against it, and the release workflow reads it to name the release.

import { execFileSync } from "node:child_process";
import { readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const SEMVER = /^\d+\.\d+\.\d+$/;

const read = (rel) => readFileSync(join(ROOT, rel), "utf8");
const write = (rel, body) => writeFileSync(join(ROOT, rel), body);

/** The [package] table of Cargo.toml, so a dependency's version is never touched. */
function packageSection(src) {
  const start = src.indexOf("[package]");
  if (start === -1) throw new Error("Cargo.toml has no [package] table");
  const after = src.indexOf("\n[", start + 1);
  return { start, end: after === -1 ? src.length : after };
}

const SITES = [
  {
    file: "package.json",
    get: (s) => JSON.parse(s).version,
    set: (s, v) => JSON.stringify({ ...JSON.parse(s), version: v }, null, 2) + "\n",
  },
  {
    file: "src-tauri/tauri.conf.json",
    authoritative: true,
    get: (s) => JSON.parse(s).version,
    set: (s, v) => JSON.stringify({ ...JSON.parse(s), version: v }, null, 2) + "\n",
  },
  {
    file: "src-tauri/Cargo.toml",
    get: (s) => {
      const { start, end } = packageSection(s);
      return s.slice(start, end).match(/^version\s*=\s*"([^"]*)"/m)?.[1] ?? null;
    },
    set: (s, v) => {
      const { start, end } = packageSection(s);
      return s.slice(0, start) + s.slice(start, end).replace(/^version\s*=\s*"[^"]*"/m, `version = "${v}"`) + s.slice(end);
    },
  },
  {
    // Rewritten by `cargo metadata` below rather than by hand.
    file: "src-tauri/Cargo.lock",
    get: (s) => s.match(/\[\[package\]\]\nname = "git-desktools"\nversion = "([^"]*)"/)?.[1] ?? null,
  },
];

function report() {
  const rows = SITES.map((s) => ({ ...s, value: s.get(read(s.file)) }));
  const width = Math.max(...rows.map((r) => r.file.length));
  for (const r of rows) console.log(`  ${r.file.padEnd(width)}  ${r.value ?? "MISSING"}${r.authoritative ? " (authoritative)" : ""}`);
  return rows;
}

const args = process.argv.slice(2);
const next = args.find((a) => !a.startsWith("-"));

if (args.includes("--check")) {
  const distinct = new Set(report().map((r) => r.value));
  if (distinct.size !== 1 || distinct.has(null)) {
    console.error(`\nMISMATCH: ${[...distinct].join(", ")} — run: node scripts/bump-version.mjs <x.y.z>`);
    process.exit(1);
  }
  console.log(`\nAll sites agree on ${[...distinct][0]}.`);
  process.exit(0);
}

if (!next || !SEMVER.test(next)) {
  console.error("usage: node scripts/bump-version.mjs <x.y.z> | --check");
  process.exit(2);
}

for (const s of SITES) if (s.set) write(s.file, s.set(read(s.file), next));
// Resolution is what rewrites the lockfile; `--no-deps` would skip it.
execFileSync("cargo", ["metadata", "--format-version", "1"], {
  cwd: join(ROOT, "src-tauri"),
  stdio: ["ignore", "ignore", "inherit"],
});

const wrong = report().filter((r) => r.value !== next);
if (wrong.length) {
  console.error(`\nFAILED: ${wrong.map((r) => r.file).join(", ")} did not reach ${next}`);
  process.exit(1);
}
console.log(`\nAll sites set to ${next}. Commit, then: git tag v${next} && git push origin v${next}`);
