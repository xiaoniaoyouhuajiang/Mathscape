import { createServer } from "node:http";
import { randomBytes, createHash } from "node:crypto";
import { networkInterfaces } from "node:os";
import { readFile, mkdir, open, link, unlink, stat } from "node:fs/promises";
import { resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { parseInkDocument, MAX_INK_BYTES, projectInk } from "../dist-ink/ink-contract.js";

const frontend = fileURLToPath(new URL("../", import.meta.url));
const privateHeaders = {
  "Cache-Control": "no-store", "X-Content-Type-Options": "nosniff", "Referrer-Policy": "no-referrer",
  "Content-Security-Policy": "default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data: blob:; connect-src 'self'; object-src 'none'; base-uri 'none'; frame-ancestors 'none'",
};

export async function startInkLab(options = {}) {
  const host = options.host ?? "0.0.0.0";
  const port = options.port ?? 1440;
  const staticDir = options.staticDir ?? resolve(frontend, "dist-ink");
  const outputDir = options.outputDir ?? resolve(frontend, "../.mathscape-dev/ink-lab/exports");
  await mkdir(outputDir, { recursive: true });
  const index = await readFile(resolve(staticDir, "index.html"));
  const hostToken = randomBytes(32).toString("hex"), padToken = randomBytes(32).toString("hex");
  const addresses = [...new Set(Object.values(networkInterfaces()).flat().filter(x => x && !x.internal && x.family === "IPv4").map(x => x.address))];
  const allowedHosts = new Set(["localhost", "127.0.0.1", "[::1]", ...addresses]);
  const exports = new Map();
  let actualPort = port;
  const json = (res, status, value) => {
    res.writeHead(status, { ...privateHeaders, "Content-Type": "application/json; charset=utf-8" });
    res.end(JSON.stringify(value));
  };
  const auth = (req, token) => req.headers.authorization === `Bearer ${token}`;
  const info = () => ({
    version: "mathscape-ink-lab-server-1",
    pad_urls: [...addresses, "127.0.0.1"].map(address => `http://${address}:${actualPort}/pad#session=${padToken}`),
    output_directory: outputDir, exports: [...exports.values()],
  });
  const server = createServer(async (req, res) => {
    try {
      const authority = req.headers.host ?? "";
      let parsed;
      try { parsed = new URL(req.url ?? "/", `http://${authority}`); } catch { return json(res, 400, { error: "无效地址" }); }
      if (!allowedHosts.has(parsed.hostname) || parsed.port !== String(actualPort)) return json(res, 403, { error: "未授权的主机地址" });
      if (req.headers.origin && req.headers.origin !== `http://${authority}`) return json(res, 403, { error: "请求来源不匹配" });
      const path = parsed.pathname;
      if (path === "/api/lab") {
        if (req.method !== "GET") return json(res, 405, { error: "仅支持 GET" });
        if (!auth(req, hostToken)) return json(res, 401, { error: "请从终端提供的桌面地址打开实验" });
        return json(res, 200, info());
      }
      if (path === "/api/pad") {
        if (req.method !== "GET") return json(res, 405, { error: "仅支持 GET" });
        if (!auth(req, padToken)) return json(res, 401, { error: "画板链接已失效，请重新扫码" });
        return json(res, 200, { version: "mathscape-ink-lab-server-1" });
      }
      if (path === "/api/export") {
        if (req.method !== "POST") return json(res, 405, { error: "仅支持 POST" });
        if (!auth(req, padToken)) return json(res, 401, { error: "画板链接已失效，请重新扫码" });
        if (!(req.headers["content-type"] ?? "").startsWith("application/json")) return json(res, 415, { error: "需要 Ink JSON" });
        if (Number(req.headers["content-length"]) > MAX_INK_BYTES) return json(res, 413, { error: "文件超过 64 MiB" });
        const chunks = []; let size = 0;
        for await (const chunk of req) {
          size += chunk.length;
          if (size > MAX_INK_BYTES) { json(res, 413, { error: "文件超过 64 MiB" }); req.resume(); return; }
          chunks.push(chunk);
        }
        let document;
        try { document = parseInkDocument(JSON.parse(Buffer.concat(chunks).toString("utf8"))); }
        catch (error) { return json(res, 422, { error: error instanceof Error ? error.message : "无法读取文件" }); }
        const bytes = Buffer.from(JSON.stringify(document));
        const hash = createHash("sha256").update(bytes).digest("hex");
        const filename = `ink-${hash}.json`, target = resolve(outputDir, filename);
        const temporary = resolve(outputDir, `.pending-${randomBytes(16).toString("hex")}`);
        let file;
        try {
          file = await open(temporary, "wx", 0o600); await file.writeFile(bytes); await file.sync(); await file.close(); file = undefined;
          try { await link(temporary, target); }
          catch (error) {
            if (error.code !== "EEXIST") throw error;
            const existing = await readFile(target);
            if (createHash("sha256").update(existing).digest("hex") !== hash) throw new Error("已有文件完整性检查失败");
          }
          const directory = await open(outputDir, "r");
          try { await directory.sync(); } finally { await directory.close(); }
        } finally {
          await file?.close(); await unlink(temporary).catch(() => {});
        }
        const projection = projectInk(document);
        const receipt = { hash, filename, title: document.title, document_id: document.document_id,
          point_count: projection.pointCount, stroke_count: projection.strokes.length,
          visible_count: projection.visible.length, saved_at: new Date().toISOString() };
        exports.set(hash, receipt);
        return json(res, 200, receipt);
      }
      const download = /^\/api\/files\/([a-f0-9]{64})$/.exec(path);
      if (download) {
        if (req.method !== "GET") return json(res, 405, { error: "仅支持 GET" });
        if (!auth(req, hostToken)) return json(res, 401, { error: "需要桌面凭据" });
        if (!exports.has(download[1])) return json(res, 404, { error: "本次实验没有该文件" });
        const bytes = await readFile(resolve(outputDir, exports.get(download[1]).filename));
        if (createHash("sha256").update(bytes).digest("hex") !== download[1]) return json(res, 409, { error: "文件完整性检查失败" });
        res.writeHead(200, { ...privateHeaders, "Content-Type": "application/json", "Content-Disposition": `attachment; filename="ink-${download[1]}.json"` });
        return res.end(bytes);
      }
      if (req.method !== "GET" && req.method !== "HEAD") return json(res, 405, { error: "不支持的请求" });
      if (["/", "/host", "/pad"].includes(path)) {
        res.writeHead(200, { ...privateHeaders, "Content-Type": "text/html; charset=utf-8" }); return res.end(req.method === "HEAD" ? undefined : index);
      }
      if (/^\/assets\/[a-zA-Z0-9_.-]+\.(js|css)$/.test(path)) {
        const filename = resolve(staticDir, path.slice(1));
        if ((await stat(filename)).size > 8 * 1024 * 1024) return json(res, 413, { error: "资源过大" });
        res.writeHead(200, { ...privateHeaders, "Content-Type": path.endsWith(".js") ? "text/javascript; charset=utf-8" : "text/css; charset=utf-8" });
        return res.end(req.method === "HEAD" ? undefined : await readFile(filename));
      }
      return json(res, 404, { error: "没有该实验资源" });
    } catch (error) {
      if (res.headersSent) { res.destroy(); return; }
      if (error.code === "ENOENT" && (req.url ?? "").startsWith("/assets/")) return json(res, 404, { error: "没有该实验资源" });
      // Do not expose host filesystem paths or credentials in remote errors.
      return json(res, 500, { error: "电脑保存失败；请先在平板下载 Ink JSON，再检查实验目录" });
    }
  });
  server.requestTimeout = 120_000; server.headersTimeout = 15_000;
  await new Promise((resolve, reject) => { server.once("error", reject); server.listen(port, host, resolve); });
  actualPort = server.address().port;
  return { server, hostToken, padToken, port: actualPort, outputDir,
    hostUrl: `http://127.0.0.1:${actualPort}/host#host=${hostToken}`,
    close: () => new Promise(resolve => { server.close(resolve); server.closeAllConnections(); }) };
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  const rawPort = process.env.INK_LAB_PORT ?? "1440";
  if (!/^\d+$/.test(rawPort) || Number(rawPort) < 1 || Number(rawPort) > 65535) throw new Error("INK_LAB_PORT 必须为 1–65535");
  const lab = await startInkLab({ port: Number(rawPort), host: process.env.INK_LAB_BIND ?? "0.0.0.0" });
  console.log(`\nMathscape Ink Lab P0\n桌面实验页面：${lab.hostUrl}\n文件目录：${lab.outputDir}\n打开桌面页面，选择与平板同一局域网的地址并扫码。Ctrl+C 关闭实验服务。\n`);
  for (const signal of ["SIGINT", "SIGTERM"]) process.once(signal, () => void lab.close().then(() => process.exit(0)));
}
