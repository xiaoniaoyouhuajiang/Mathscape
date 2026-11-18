/**
 * Markdown 文档侧边大纲目录（TOC）与已入图单元锚点装饰器。
 *
 * 同时支持物理快照 Markdown 与拼接式「规范解析正文」：
 * - 提取文内 ATX 标题（#..####）与后端已登记的 `SourceSemanticUnit`；
 * - 左侧提供可折叠大纲栏（并排窄栏默认收起，宽屏多条目默认展开）；
 * - 在已入图单元首块上渲染非侵入式 `◈ 标题 ↗` 徽章，一键跳回数学地图定位。
 */
import { eventBus } from "../core/events";
import { texToPlain } from "../graph/tex-plain";
import type { MarkdownUnitMarker } from "./content";

export interface OutlineBlockDescriptor {
  id: string;
  domKey: string;
  artifactId: string;
  range: { start: number; end: number };
  text: string;
  kind?: string;
}

export interface OutlineHeading {
  domKey: string;
  blockId: string;
  level: 1 | 2 | 3 | 4;
  title: string;
}

export interface MatchedUnit {
  unit: MarkdownUnitMarker;
  domKey: string;
  blockId: string;
}

export interface DocumentOutlineEntry {
  kind: "heading" | "unit";
  domKey: string;
  blockId: string;
  level: 1 | 2 | 3 | 4;
  title: string;
  unitId?: string;
  unitKind?: string;
}

const ATX_HEADING = /^\s*(#{1,4})\s+(.+?)\s*#*\s*$/m;
const OUTLINE_PREF_KEY = "mathscape:md-outline";

function cleanHeadingTitle(raw: string): string {
  const stripped = raw
    .replace(/\*\*(.+?)\*\*/g, "$1")
    .replace(/__(.+?)__/g, "$1")
    .replace(/`([^`]+)`/g, "$1")
    .replace(/\s+/g, " ")
    .trim();
  return texToPlain(stripped) || stripped;
}

/** Extracts ATX `#..####` headings from non-page-marker blocks (ignoring fenced code blocks). */
export function extractMarkdownHeadings(
  blocks: readonly Pick<OutlineBlockDescriptor, "id" | "domKey" | "text" | "kind">[],
): OutlineHeading[] {
  const out: OutlineHeading[] = [];
  for (const block of blocks) {
    if (block.kind === "page_marker" || block.kind === "code") continue;
    const trimmed = block.text.trimStart();
    if (trimmed.startsWith("```") || trimmed.startsWith("~~~")) continue;
    const firstLine = trimmed.split("\n")[0] ?? "";
    const match = ATX_HEADING.exec(firstLine);
    if (!match) continue;
    const level = Math.min(4, Math.max(1, match[1].length)) as 1 | 2 | 3 | 4;
    const title = cleanHeadingTitle(match[2]);
    if (!title) continue;
    out.push({ domKey: block.domKey, blockId: block.id, level, title });
  }
  return out;
}

/** Matches registered `SourceSemanticUnit` markers to their first overlapping content block. */
export function matchUnitsToBlocks(
  units: readonly MarkdownUnitMarker[],
  blocks: readonly Pick<OutlineBlockDescriptor, "id" | "domKey" | "artifactId" | "range" | "kind">[],
): MatchedUnit[] {
  const out: MatchedUnit[] = [];
  for (const unit of units) {
    const wantedIds = unit.block_ids.length ? new Set(unit.block_ids) : null;
    const hitById = wantedIds
      ? blocks.find((b) => b.kind !== "page_marker" && b.artifactId === unit.artifact_id && wantedIds.has(b.id))
      : undefined;
    const hit =
      hitById ??
      blocks.find(
        (block) =>
          block.kind !== "page_marker" &&
          block.artifactId === unit.artifact_id &&
          unit.byte_range.end > unit.byte_range.start &&
          block.range.start < unit.byte_range.end &&
          block.range.end > unit.byte_range.start,
      );
    if (hit) {
      out.push({ unit, domKey: hit.domKey, blockId: hit.id });
    }
  }
  return out;
}

/** Merges headings and registered units in exact document block order, deduplicating identical section blocks. */
export function buildDocumentOutline(
  headings: readonly OutlineHeading[],
  matchedUnits: readonly MatchedUnit[],
  domKeyOrder: readonly string[],
): DocumentOutlineEntry[] {
  const orderMap = new Map(domKeyOrder.map((key, index) => [key, index]));
  const headingByKey = new Map(headings.map((h) => [h.domKey, h]));
  const unitsByKey = new Map<string, MatchedUnit[]>();
  for (const m of matchedUnits) {
    const arr = unitsByKey.get(m.domKey) ?? [];
    arr.push(m);
    unitsByKey.set(m.domKey, arr);
  }

  const allKeys = new Set<string>([...headingByKey.keys(), ...unitsByKey.keys()]);
  const sortedKeys = [...allKeys].sort((a, b) => (orderMap.get(a) ?? 0) - (orderMap.get(b) ?? 0));

  const entries: DocumentOutlineEntry[] = [];
  for (const key of sortedKeys) {
    const heading = headingByKey.get(key);
    const units = unitsByKey.get(key) ?? [];
    if (units.length === 0 && heading) {
      entries.push({
        kind: "heading",
        domKey: key,
        blockId: heading.blockId,
        level: heading.level,
        title: heading.title,
      });
      continue;
    }
    const mergesHeading = Boolean(
      heading &&
        units.some((m) => {
          const t = texToPlain(m.unit.title) || m.unit.title;
          return m.unit.unit_kind === "section" || t === heading.title;
        }),
    );
    if (heading && !mergesHeading) {
      entries.push({
        kind: "heading",
        domKey: key,
        blockId: heading.blockId,
        level: heading.level,
        title: heading.title,
      });
    }
    let consumedHeading = false;
    for (const m of units) {
      const unitTitle = texToPlain(m.unit.title) || m.unit.title;
      const isMergedSection =
        Boolean(heading && !consumedHeading && (m.unit.unit_kind === "section" || unitTitle === heading.title));
      const level: 1 | 2 | 3 | 4 = isMergedSection && heading
        ? heading.level
        : m.unit.unit_kind === "section"
          ? 2
          : 3;
      if (isMergedSection) consumedHeading = true;
      entries.push({
        kind: "unit",
        domKey: key,
        blockId: m.blockId,
        level,
        title: unitTitle,
        unitId: m.unit.unit_id,
        unitKind: m.unit.unit_kind,
      });
    }
  }
  return entries;
}

export interface MarkdownOutlineHandle {
  readonly toggleButton: HTMLButtonElement;
  readonly nav: HTMLElement;
  update(blocks: readonly OutlineBlockDescriptor[], units: readonly MarkdownUnitMarker[]): void;
  setHiddenForRaw(raw: boolean): void;
  dispose(): void;
}

export function mountMarkdownOutline(
  article: HTMLElement,
  resolveBlockElement: (domKey: string) => HTMLElement | null,
): MarkdownOutlineHandle {
  const toggleButton = document.createElement("button");
  toggleButton.type = "button";
  toggleButton.textContent = "目录";
  toggleButton.title = "显示或隐藏章节大纲与已入图单元目录";

  const nav = document.createElement("nav");
  nav.className = "md-outline";
  nav.setAttribute("aria-label", "文档大纲目录");

  let userToggled = false;
  let visible = false;
  let rawMode = false;
  let currentEntries: DocumentOutlineEntry[] = [];
  let focusTimer: number | undefined;

  const readStoredPref = (): boolean | null => {
    try {
      const v = localStorage.getItem(OUTLINE_PREF_KEY);
      return v === "1" ? true : v === "0" ? false : null;
    } catch {
      return null;
    }
  };

  const inSplit = () => Boolean(article.closest("#ink-split-reader"));

  const applyVisibility = (): void => {
    const effective = !rawMode && visible;
    nav.hidden = !effective;
    toggleButton.hidden = rawMode;
    toggleButton.setAttribute("aria-pressed", String(effective));
  };

  const syncDefaultVisibility = (): void => {
    if (userToggled) {
      applyVisibility();
      return;
    }
    if (inSplit() || window.innerWidth < 1060) {
      visible = false;
    } else {
      const stored = readStoredPref();
      visible = currentEntries.length >= 2 && stored !== false;
    }
    applyVisibility();
  };

  toggleButton.onclick = () => {
    userToggled = true;
    visible = !visible;
    try {
      localStorage.setItem(OUTLINE_PREF_KEY, visible ? "1" : "0");
    } catch {
      /* Optional preference */
    }
    applyVisibility();
  };

  const onResize = () => syncDefaultVisibility();
  window.addEventListener("resize", onResize);

  const decorateUnitBadges = (matched: readonly MatchedUnit[]): void => {
    article.querySelectorAll(".unit-map-badge-row").forEach((el) => el.remove());
    if (rawMode || !matched.length) return;
    const byKey = new Map<string, MatchedUnit[]>();
    for (const item of matched) {
      const list = byKey.get(item.domKey) ?? [];
      list.push(item);
      byKey.set(item.domKey, list);
    }
    for (const [domKey, list] of byKey) {
      const blockEl = resolveBlockElement(domKey);
      if (!blockEl || blockEl.classList.contains("source-page-marker")) continue;
      const row = document.createElement("div");
      row.className = "unit-map-badge-row";
      const stop = (e: Event) => e.stopPropagation();
      row.addEventListener("click", stop);
      row.addEventListener("pointerdown", stop);
      row.addEventListener("contextmenu", stop);
      row.addEventListener("keydown", stop);
      for (const { unit } of list) {
        const label = texToPlain(unit.title) || unit.title;
        const badge = document.createElement("button");
        badge.type = "button";
        badge.className = "unit-map-badge";
        badge.textContent = `◈ ${label} ↗`;
        badge.title = "已在数学地图登记，点击在主地图中定位此单元";
        badge.onclick = (e) => {
          e.stopPropagation();
          eventBus.emit("locate-domain-object", { objectId: unit.unit_id, title: label });
        };
        row.append(badge);
      }
      blockEl.prepend(row);
    }
  };

  const renderOutlineList = (): void => {
    nav.replaceChildren();
    const header = document.createElement("div");
    header.className = "md-outline-header";
    header.textContent = `目录与入图锚点（${currentEntries.length}）`;
    nav.append(header);

    if (!currentEntries.length) {
      const empty = document.createElement("p");
      empty.className = "md-outline-empty";
      empty.textContent = "本文档暂无章节标题或已入图单元。";
      nav.append(empty);
      return;
    }

    const list = document.createElement("div");
    list.className = "md-outline-list";
    for (const entry of currentEntries) {
      const row = document.createElement("div");
      row.className = `md-outline-item md-outline-level-${entry.level}${entry.kind === "unit" ? " is-registered-unit" : ""}`;

      const jumpBtn = document.createElement("button");
      jumpBtn.type = "button";
      jumpBtn.className = "md-outline-jump";
      jumpBtn.textContent = entry.kind === "unit" ? `◈ ${entry.title}` : entry.title;
      jumpBtn.title = entry.kind === "unit" ? `${entry.title}（已入图单元，点击滚动定位）` : entry.title;
      jumpBtn.onclick = () => {
        const target = resolveBlockElement(entry.domKey);
        if (!target) return;
        article.querySelectorAll(".is-outline-pulse").forEach((el) => el.classList.remove("is-outline-pulse"));
        target.classList.add("is-outline-pulse");
        target.scrollIntoView({ behavior: "smooth", block: "center" });
        if (focusTimer) window.clearTimeout(focusTimer);
        focusTimer = window.setTimeout(() => target.classList.remove("is-outline-pulse"), 1800);
      };
      row.append(jumpBtn);

      if (entry.unitId) {
        const mapBtn = document.createElement("button");
        mapBtn.type = "button";
        mapBtn.className = "md-outline-map-link";
        mapBtn.textContent = "地图 ↗";
        mapBtn.title = "在数学地图中定位此单元";
        mapBtn.onclick = (e) => {
          e.stopPropagation();
          eventBus.emit("locate-domain-object", { objectId: entry.unitId!, title: entry.title });
        };
        row.append(mapBtn);
      }
      list.append(row);
    }
    nav.append(list);
  };

  return {
    toggleButton,
    nav,
    update(blocks, units) {
      const headings = extractMarkdownHeadings(blocks);
      const matched = matchUnitsToBlocks(units, blocks);
      currentEntries = buildDocumentOutline(
        headings,
        matched,
        blocks.map((b) => b.domKey),
      );
      renderOutlineList();
      decorateUnitBadges(matched);
      syncDefaultVisibility();
    },
    setHiddenForRaw(raw) {
      rawMode = raw;
      applyVisibility();
    },
    dispose() {
      if (focusTimer) window.clearTimeout(focusTimer);
      window.removeEventListener("resize", onResize);
      nav.remove();
      toggleButton.remove();
    },
  };
}
