#!/usr/bin/env node
// Counts source lines under frontend/src that carry Chinese text outside comments
// (src/i18n is excluded) and fails when the count rises above the baseline.
// New UI text goes through t() in src/i18n; `--update` lowers the baseline after a migration.
import { readdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, relative, sep } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const srcDir = join(root, "src");
const baselinePath = join(root, "i18n-baseline.json");
const CJK = /[\u3400-\u4dbf\u4e00-\u9fff\uf900-\ufaff]/u;

function* sourceFiles(dir) {
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) {
      if (relative(srcDir, path) !== "i18n") yield* sourceFiles(path);
    } else if (entry.name.endsWith(".ts")) yield path;
  }
}

/** Replaces comment text with spaces, keeping strings, template literals and line breaks. */
function stripComments(text) {
  let out = "";
  let i = 0;
  let quote = null;
  while (i < text.length) {
    const c = text[i];
    const next = text[i + 1];
    if (quote) {
      out += c;
      if (c === "\\") { out += next ?? ""; i += 2; continue; }
      if (c === quote) quote = null;
      i += 1;
    } else if (c === "/" && next === "/") {
      while (i < text.length && text[i] !== "\n") { out += " "; i += 1; }
    } else if (c === "/" && next === "*") {
      const end = text.indexOf("*/", i + 2);
      const stop = end === -1 ? text.length : end + 2;
      out += text.slice(i, stop).replace(/[^\n]/g, " ");
      i = stop;
    } else {
      if (c === "\"" || c === "'" || c === "`") quote = c;
      out += c;
      i += 1;
    }
  }
  return out;
}

function countCjkLines() {
  const files = {};
  let total = 0;
  for (const path of sourceFiles(srcDir)) {
    const lines = stripComments(readFileSync(path, "utf8")).split("\n").filter(line => CJK.test(line)).length;
    if (lines > 0) files[relative(root, path).split(sep).join("/")] = lines;
    total += lines;
  }
  return { total, files };
}

const { total } = countCjkLines();
const baseline = JSON.parse(readFileSync(baselinePath, "utf8"));

if (process.argv.includes("--update")) {
  if (total > baseline.cjkLines) {
    console.error(`i18n ratchet: ${total} lines exceed the baseline ${baseline.cjkLines}; the baseline only goes down.`);
    process.exit(1);
  }
  writeFileSync(baselinePath, `${JSON.stringify({ cjkLines: total }, null, 2)}\n`);
  console.log(`i18n ratchet: baseline set to ${total}.`);
} else if (total > baseline.cjkLines) {
  console.error(`i18n ratchet: ${total} lines in frontend/src carry Chinese text, baseline ${baseline.cjkLines}.`);
  console.error("Put new UI text in src/i18n (zh-CN.ts and en.ts) and call t(key).");
  process.exit(1);
} else if (total < baseline.cjkLines) {
  console.log(`i18n ratchet: ${total} lines, below the baseline ${baseline.cjkLines}; run \`npm run check:i18n -- --update\` to lock it in.`);
} else {
  console.log(`i18n ratchet: ${total} lines, at the baseline.`);
}
