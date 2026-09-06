import { invoke } from "@tauri-apps/api/core";

export interface ProductPath { path: string; artifact_id: string }
export interface ContentInfo {
  artifact_id: string;
  media_type: string;
  byte_size: number;
  contexts: { conversion_id: string; paths: ProductPath[] }[];
}

export function contentInfo(id: string): Promise<ContentInfo> {
  return invoke("get_artifact_content_info", { artifactId: id });
}

export async function readRange(id: string, offset: number, length: number): Promise<Uint8Array> {
  const result = new Uint8Array(length);
  for (let position = 0; position < length;) {
    const count = Math.min(4 * 1024 * 1024, length - position);
    const data = await invoke<ArrayBuffer>("read_artifact_bytes", { artifactId: id, offset: offset + position, length: count });
    const chunk = new Uint8Array(data);
    if (chunk.byteLength !== count) throw new Error("文件长度已改变，请重新打开。");
    result.set(chunk, position);
    position += count;
  }
  return result;
}

export async function readSmallFile(info: ContentInfo): Promise<Uint8Array> {
  if (info.byte_size > 16 * 1024 * 1024) throw new Error("文本或图片超过 16 MiB，当前阅读视图暂不加载。");
  return readRange(info.artifact_id, 0, info.byte_size);
}

// Resolve against the conversion's virtual root, never against a host path.
export function resolveProductPath(base: string, reference: string): string | null {
  let ref: string;
  try { ref = decodeURIComponent(reference.split(/[?#]/)[0]); } catch { return null; }
  if (!ref || /^[a-z][a-z0-9+.-]*:/i.test(ref) || ref.startsWith("/") || ref.includes("\\") || /[\u0000-\u001f]/.test(ref)) return null;
  const parts = base.split("/").slice(0, -1);
  for (const part of ref.split("/")) {
    if (part === "..") { if (!parts.length) return null; parts.pop(); }
    else if (part && part !== ".") parts.push(part);
  }
  return parts.join("/");
}
