import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, readFile, readdir, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { startInkLab } from "../scripts/ink-lab-server.mjs";

test("the P0 harness authenticates, validates, saves and retries complete Ink files", async () => {
  const directory = await mkdtemp(join(tmpdir(), "mathscape-ink-"));
  const lab = await startInkLab({ host: "127.0.0.1", port: 0, outputDir: directory });
  const base = `http://127.0.0.1:${lab.port}`;
  const trace = JSON.parse(await readFile(new URL("../../testdata/ink/p0/trace.json", import.meta.url), "utf8"));
  const send = (body, token = lab.padToken, headers = {}) => fetch(`${base}/api/export`, {
    method: "POST", headers: { Authorization: `Bearer ${token}`, "Content-Type": "application/json", ...headers }, body: JSON.stringify(body),
  });
  try {
    assert.equal((await fetch(`${base}/api/lab`)).status, 401);
    assert.equal((await send(trace, "invalid")).status, 401);
    assert.equal((await send(trace, lab.padToken, { Origin: "http://untrusted.invalid" })).status, 403);
    const invalid = structuredClone(trace); invalid.operations[1].seq = 42;
    assert.equal((await send(invalid)).status, 422);
    assert.deepEqual(await readdir(directory), []);
    const first = await send(trace); assert.equal(first.status, 200);
    const receipt = await first.json(); assert.equal(receipt.point_count, 7);
    assert.deepEqual(JSON.parse(await readFile(join(directory, receipt.filename), "utf8")), trace);
    const retry = await send(trace); assert.equal(retry.status, 200); assert.equal((await retry.json()).hash, receipt.hash);
    assert.equal((await readdir(directory)).length, 1);
    assert.equal((await fetch(`${base}/api/files/${receipt.hash}`)).status, 401);
    const downloaded = await fetch(`${base}/api/files/${receipt.hash}`, { headers: { Authorization: `Bearer ${lab.hostToken}` } });
    assert.equal(downloaded.status, 200); assert.deepEqual(await downloaded.json(), trace);
    const info = await fetch(`${base}/api/lab`, { headers: { Authorization: `Bearer ${lab.hostToken}` } });
    assert.equal((await info.json()).exports.length, 1);
    for (const path of ["/.env", "/src/main.ts", "/ink-contract.js", "/assets/not-found.js"]) assert.equal((await fetch(`${base}${path}`)).status, 404);
  } finally { await lab.close(); await rm(directory, { recursive: true }); }
});

test("write failures do not return a successful save receipt", async () => {
  const directory = await mkdtemp(join(tmpdir(), "mathscape-ink-"));
  const lab = await startInkLab({ host: "127.0.0.1", port: 0, outputDir: directory });
  try {
    await rm(directory, { recursive: true });
    const trace = await readFile(new URL("../../testdata/ink/p0/trace.json", import.meta.url), "utf8");
    const response = await fetch(`http://127.0.0.1:${lab.port}/api/export`, { method: "POST",
      headers: { Authorization: `Bearer ${lab.padToken}`, "Content-Type": "application/json" }, body: trace });
    assert.equal(response.status, 500);
    assert.match((await response.json()).error, /保存失败/);
  } finally { await lab.close(); }
});
