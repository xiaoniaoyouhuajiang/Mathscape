/**
 * 地图标签悬停：展开全文、描边光晕、提到最上层，并暂时藏起会挡住它的邻接标签。
 *
 * Cytoscape 3.34 只改 `label` 数据或 class 时，标签的包围盒和纹理仍可能沿用旧文字的
 * 宽度，新标签会被裁成旧宽度（悬停截图里「定理 3.2.3 Cassini…」左侧缺字、没有深色底）。
 * 换字后要作废包围盒缓存并 `updateStyle()`，等两帧再量一次。
 */
import type { CollectionReturnValue, Core, EventObject, NodeSingular } from "cytoscape";

export const LABEL_HOVER = "label-hover";
export const LABEL_RECESS = "label-recess";
const RELATED_EDGE_HOVER = "is-theme-hover";

/** 焦点 / 悬停共用的标签层级，并压在所有其他标签之上。 */
export const LABEL_HOVER_STYLE = {
  "text-opacity": 1,
  "text-events": "yes",
  "text-wrap": "wrap",
  "text-background-opacity": 0,
  "text-background-shape": "roundrectangle",
  "text-background-padding": "3px",
  "text-border-width": 0,
  "text-border-opacity": 1,
  opacity: 1,
  "z-index": 1000,
  "z-index-compare": "manual",
  "z-compound-depth": "top",
} as const;

/** 会与悬停标签重叠的邻接标签：暂时藏起文字和底，移出后恢复。 */
export const LABEL_RECESS_STYLE = {
  "text-opacity": 0,
  "text-events": "no",
  "text-background-opacity": 0,
  "text-border-opacity": 0,
} as const;

export type LabelBox = { x1: number; x2: number; y1: number; y2: number; w: number; h: number };

/** Cytoscape 的节点 / 边 / 集合在类型上不完全互通，这里只用到这些方法。 */
export type LabelEle = {
  id(): string;
  visible(): boolean;
  removed(): boolean;
  isNode(): boolean;
  same(other: LabelEle): boolean;
  hasClass(name: string): boolean;
  addClass(name: string): unknown;
  data(name: string): unknown;
  style(name: string): string;
  boundingBox(opts: object): LabelBox;
};

export type LabelSet = {
  empty(): boolean;
  boundingBox(opts: object): unknown;
  dirtyBoundingBoxCache?: () => void;
  updateStyle(): void;
  forEach(fn: (ele: LabelEle) => void): void;
};

const LABEL_BOX_OPTS = {
  includeNodes: false,
  includeEdges: false,
  includeLabels: true,
  includeOverlays: false,
  includeUnderlays: false,
} as const;

export function labelBox(ele: LabelEle): LabelBox {
  return ele.boundingBox(LABEL_BOX_OPTS);
}

export function boxesOverlap(a: LabelBox, b: LabelBox): boolean {
  return a.x1 < b.x2 && b.x1 < a.x2 && a.y1 < b.y2 && b.y1 < a.y2;
}

/** 焦点或悬停用全文，其余用短名。直接改 `label`，让 Cytoscape 重算标签尺寸。 */
export function syncNodeLabel(node: NodeSingular, full: boolean): boolean {
  const rawBase = String(node.data(full ? "fullLabel" : "compactLabel") ?? node.data("inkBaseLabel") ?? node.data("errataBaseLabel") ?? node.data("label") ?? "");
  const inkCount = Number(node.data("inkCount") ?? 0);
  const errataCount = Number(node.data("errataUnreviewedCount") ?? 0);
  const parts: string[] = [];
  if (inkCount > 0) parts.push(`✎${inkCount}`);
  if (errataCount > 0) parts.push("已校对");
  const suffix = parts.length ? ` · ${parts.join(" · ")}` : "";
  const prefix = node.data("elementKind") === "junction" ? "" : node.hasClass("is-staged") ? "〔未审批〕" : node.hasClass("is-staged-conflict") ? "〔提案冲突〕" : "";
  const base = rawBase.replace(/^〔(未审批|提案冲突)〕/, "");
  const next = prefix + (suffix && !base.endsWith(suffix) ? `${base}${suffix}` : base);
  if (node.data("label") === next) return false;
  node.data("label", next);
  return true;
}

type LabelCacheScratch = {
  prefixedLabelDimsKey?: unknown;
  labelWrapKey?: unknown;
  labelWrapCachedText?: unknown;
  labelWrapCachedLines?: unknown;
  labelWidth?: unknown;
  labelHeight?: unknown;
  sourcePrefixedLabelDimsKey?: unknown;
  sourceLabelWrapKey?: unknown;
  sourceLabelWrapCachedText?: unknown;
  sourceLabelWidth?: unknown;
  sourceLabelHeight?: unknown;
  targetPrefixedLabelDimsKey?: unknown;
  targetLabelWrapKey?: unknown;
  targetLabelWrapCachedText?: unknown;
  targetLabelWidth?: unknown;
  targetLabelHeight?: unknown;
};

function invalidateLabelCache(eles: LabelSet): void {
  eles.forEach((ele) => {
    const rs = (ele as unknown as { _private?: { rscratch?: LabelCacheScratch } })._private?.rscratch;
    if (!rs) return;
    delete rs.prefixedLabelDimsKey;
    delete rs.labelWrapKey;
    delete rs.labelWrapCachedText;
    delete rs.labelWrapCachedLines;
    delete rs.labelWidth;
    delete rs.labelHeight;
    delete rs.sourcePrefixedLabelDimsKey;
    delete rs.sourceLabelWrapKey;
    delete rs.sourceLabelWrapCachedText;
    delete rs.sourceLabelWidth;
    delete rs.sourceLabelHeight;
    delete rs.targetPrefixedLabelDimsKey;
    delete rs.targetLabelWrapKey;
    delete rs.targetLabelWrapCachedText;
    delete rs.targetLabelWidth;
    delete rs.targetLabelHeight;
  });
  const dirty = eles as unknown as { dirtyBoundingBoxCache(): void };
  if (typeof dirty.dirtyBoundingBoxCache === "function") dirty.dirtyBoundingBoxCache();
}

/**
 * 换过文字的节点要重算样式：先作废包围盒 / 纹理尺寸缓存，再 `updateStyle`。
 * `updateStyle` 是公开方法，只是没写进类型声明。新尺寸要等画过一帧才稳定。
 */
export function refreshLabels(core: Core, nodes: LabelSet, after?: () => void): void {
  if (nodes.empty()) {
    after?.();
    return;
  }
  const update = (): void => {
    if (core.destroyed()) return;
    invalidateLabelCache(nodes);
    (nodes as unknown as { updateStyle(): void }).updateStyle();
    nodes.boundingBox({ includeLabels: true });
  };
  update();
  requestAnimationFrame(() => requestAnimationFrame(() => {
    update();
    after?.();
  }));
}

function isLabeled(ele: LabelEle): boolean {
  if (!ele.visible()) return false;
  const text = String(ele.style("label") ?? (ele.isNode() ? ele.data("label") : "") ?? "");
  return text !== "";
}

function recessColliding(core: Core, hovered: LabelEle): void {
  if (core.destroyed() || hovered.removed() || !hovered.hasClass(LABEL_HOVER)) return;
  const hover = labelBox(hovered);
  if (hover.w <= 0 || hover.h <= 0) return;
  let recede: CollectionReturnValue = core.collection();
  core.elements().forEach((ele) => {
    const item = ele as unknown as LabelEle;
    if (item.same(hovered) || item.hasClass(LABEL_RECESS) || !isLabeled(item)) return;
    if (boxesOverlap(hover, labelBox(item))) recede = recede.union(ele);
  });
  if (recede.empty()) return;
  recede.addClass(LABEL_RECESS);
}

export interface LabelHoverOptions {
  /** 即使没有悬停也显示全文（例如教程的焦点节点）。 */
  isFull?: (node: NodeSingular) => boolean;
}

/**
 * 给已挂载的地图绑定悬停：有 `fullLabel` / `compactLabel` 时展开全文，
 * 提到最上层；会重叠的邻接标签暂时藏起。
 */
export function bindLabelHover(core: Core, options: LabelHoverOptions = {}): void {
  const isFull = options.isFull ?? (() => false);
  let token = 0;

  const applyNodeLabel = (node: NodeSingular): boolean =>
    syncNodeLabel(node, node.hasClass(LABEL_HOVER) || isFull(node));

  const clear = (): void => {
    if (core.destroyed()) return;
    const els = core.elements(`.${LABEL_HOVER}, .${LABEL_RECESS}, .${RELATED_EDGE_HOVER}`);
    if (els.empty()) return;
    token += 1;
    const nodes = els.nodes();
    core.batch(() => {
      els.removeClass(`${LABEL_HOVER} ${LABEL_RECESS} ${RELATED_EDGE_HOVER}`);
      nodes.forEach((node) => applyNodeLabel(node));
    });
    refreshLabels(core, nodes as unknown as LabelSet);
  };

  const enter = (target: LabelEle): void => {
    if (target.isNode() && target.data("elementKind") === "junction") return;
    if (!target.visible()) return;
    clear();
    const my = ++token;
    target.addClass(LABEL_HOVER);
    // One transient owner for the title and its connected edges. A separate
    // mouseover listener could otherwise outlive canvas exit/pan/zoom cleanup.
    if (target.isNode()) (target as NodeSingular).connectedEdges().addClass(RELATED_EDGE_HOVER);
    const nodes = (target.isNode() ? target : core.collection()) as unknown as LabelSet;
    if (target.isNode()) applyNodeLabel(target as NodeSingular);
    refreshLabels(core, nodes, () => {
      if (token !== my || core.destroyed()) return;
      recessColliding(core, target);
    });
  };

  core.on("mouseover", "node, edge", (event: EventObject) => {
    const target = event.target as unknown as LabelEle;
    if (target.hasClass(LABEL_HOVER)) return;
    enter(target);
  });
  core.on("mouseout", "node, edge", (event: EventObject) => {
    const target = event.target as unknown as LabelEle;
    if (!target.hasClass(LABEL_HOVER)) return;
    clear();
  });
  // Cytoscape can miss an element's mouseout when the canvas moves or the
  // pointer leaves it directly. Never retain a bright edge/expanded title then.
  core.on("pan zoom", clear);
  const container = core.container?.();
  if (container) {
    container.addEventListener("mouseleave", clear);
    container.addEventListener("pointercancel", clear);
    core.one("destroy", () => {
      container.removeEventListener("mouseleave", clear);
      container.removeEventListener("pointercancel", clear);
    });
  }
}
