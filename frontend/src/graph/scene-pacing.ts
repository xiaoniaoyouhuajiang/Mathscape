/**
 * Map Scene Player pacing (pure, no DOM): how live segments queue up.
 *
 * Live only — a replay is never paced and plays every step:
 * - a newer frame of a queued call (its result, a 「解析中」 update) replaces
 *   it in place;
 * - consecutive segments of the same tool coalesce into the newest one
 *   (`ToolScene.coalesced` counts the merged calls; the HUD says 合并 N 次);
 * - the queue is capped (oldest dropped first);
 * - when the head has waited too long the queue skips to the latest.
 * A `propose_changes` segment is never coalesced or dropped.
 */
import type { SceneAct, ToolScene } from "../sandbox/scene/types";

export type ScenePart = "call" | "result" | "full";

export interface Segment {
  scene: ToolScene;
  part: ScenePart;
  /** 1-based position in a turn replay. */
  step?: number;
  total?: number;
  /** Live pacing: when it was queued, and the calls merged into it. */
  enqueuedAt?: number;
  merged?: Set<string>;
  /** Live pacing: segments skipped to reach this one. */
  skipped?: number;
}

export const SCENE_PACING = {
  /** At most this many segments wait in the live queue. */
  queueCap: 6,
  /** Head waited longer than this → skip to the latest. */
  lagMs: 6000,
  /** Merge consecutive same-tool segments. */
  coalesce: true,
  /** Result layers beyond this are lit together with the last one. */
  layerCap: 4,
  /** Time budget per segment (waits are cut once it is spent). */
  segmentMaxMs: 7000,
  proposeMaxMs: 15000,
};

const isParsingRun = (scene: ToolScene): boolean => scene.act?.kind === "parsing" && scene.act.phase === "running";
const kept = (scene: ToolScene): boolean => scene.tool === "propose_changes";

function union(a: Set<string> | undefined, b: Set<string> | undefined): Set<string> {
  return new Set([...(a ?? []), ...(b ?? [])]);
}

/** Queue a live segment: replace / coalesce / cap. Returns the new queue. */
export function enqueueLive(queue: readonly Segment[], segment: Segment, pacing = SCENE_PACING): Segment[] {
  const next = [...queue];
  const { scene } = segment;
  const same = next.findIndex((queued) => queued.scene.callId === scene.callId);
  if (same >= 0) {
    const old = next[same];
    const part = old.part === "call" && segment.part === "call" ? "call" : segment.part;
    const merged = union(old.merged, segment.merged);
    const coalesced = old.scene.coalesced;
    next[same] = { ...segment, part, merged, ...(coalesced ? { scene: { ...scene, coalesced } } : {}) };
    return next;
  }
  const tail = next[next.length - 1];
  if (pacing.coalesce && tail && tail.scene.tool === scene.tool && !kept(scene) && !isParsingRun(scene) && !isParsingRun(tail.scene)) {
    const merged = union(tail.merged, segment.merged);
    next[next.length - 1] = { ...segment, merged, scene: { ...scene, coalesced: merged.size - 1 } };
    return next;
  }
  next.push(segment);
  while (next.length > pacing.queueCap) {
    const drop = next.findIndex((queued) => !kept(queued.scene));
    if (drop < 0 || drop === next.length - 1) break;
    next.splice(drop, 1);
  }
  return next;
}

/** Far behind live progress: keep proposals and the latest segment. */
export function catchUpLive(queue: readonly Segment[], now: number, pacing = SCENE_PACING): Segment[] {
  const head = queue[0];
  if (head?.enqueuedAt === undefined || queue.length < 2 || now - head.enqueuedAt <= pacing.lagMs) return [...queue];
  const last = queue[queue.length - 1];
  const next = queue.filter((queued) => queued === last || kept(queued.scene));
  const skipped = queue.length - next.length;
  if (skipped === 0) return [...queue];
  return next.map((queued) => (queued === last ? { ...last, skipped: (last.skipped ?? 0) + skipped } : queued));
}

/** Layers beyond `cap` are lit together with the last kept one. */
export function capLayers(layers: readonly string[][], cap: number): string[][] {
  if (layers.length <= cap) return [...layers];
  return [...layers.slice(0, cap - 1), layers.slice(cap - 1).flat()];
}

export function elapsedLabel(ms: number): string {
  const seconds = Math.max(0, ms) / 1000;
  if (seconds < 60) return `${seconds < 10 ? seconds.toFixed(1) : Math.floor(seconds)}s`;
  return `${Math.floor(seconds / 60)}m${String(Math.floor(seconds % 60)).padStart(2, "0")}s`;
}

/** Running pill text of an in-flight call. */
export function runningLabel(scene: { tool: string; title: string; act?: SceneAct }): string {
  if (scene.act?.kind === "parsing") {
    const pages = scene.act.pages;
    return `Marker 解析中${pages ? (pages.from === pages.to ? ` 第${pages.from}页` : ` 第${pages.from}–${pages.to}页`) : ""}`;
  }
  switch (scene.tool) {
    case "mathlib.search":
      return "Loogle 启动中…";
    case "lean.check":
      return "Lean 编译中…";
    case "propose_changes":
      return "提案校验中…";
    default:
      return `${scene.title.split(" · ")[0] || scene.tool} 运行中…`;
  }
}
