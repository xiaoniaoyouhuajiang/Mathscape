import type { CameraFramingBox } from "./study-posture";

export interface StudyLaneLabel {
  id: string;
  lane: 0 | 1 | 2;
  order: number;
  height: number;
  indent?: number;
}

/** Pack measured captions, not point centres. Use spare height for breathing room. */
export function layoutStudyLanes(labels: readonly StudyLaneLabel[], width: number, height: number) {
  const positions = new Map<string, { x: number; y: number }>();
  const column = width / 3;
  for (const lane of [0, 1, 2] as const) {
    const rows = labels.filter(item => item.lane === lane).sort((a, b) => a.order - b.order || a.id.localeCompare(b.id));
    const contentHeight = rows.reduce((sum, item) => sum + item.height, 0);
    const gap = rows.length > 1 ? Math.max(14, (height - contentHeight) / (rows.length - 1)) : 0;
    let y = 0;
    for (const row of rows) {
      positions.set(row.id, { x: column * (lane + 1) - 24 + Math.min(18, row.indent ?? 0), y: y + row.height / 2 });
      y += row.height + gap;
    }
  }
  return positions;
}

export interface FramedLabel {
  x: number;
  y: number;
  left: number;
  right: number;
  top: number;
  bottom: number;
}

/** Model-space tracks, screen-space captions: fitting must account for both. */
export function fitStudyViewport(box: CameraFramingBox, labels: readonly FramedLabel[], viewport: CameraFramingBox, maxZoom = 1.04) {
  const bounds = (zoom: number) => {
    let x1 = box.x1 * zoom, x2 = box.x2 * zoom;
    let y1 = box.y1 * zoom, y2 = box.y2 * zoom;
    for (const label of labels) {
      x1 = Math.min(x1, label.x * zoom - label.left);
      x2 = Math.max(x2, label.x * zoom + label.right);
      if (label.y >= box.y1 && label.y <= box.y2) {
        y1 = Math.min(y1, label.y * zoom - label.top);
        y2 = Math.max(y2, label.y * zoom + label.bottom);
      }
    }
    return { x1, x2, y1, y2 };
  };
  let lo = 0, hi = maxZoom;
  for (let i = 0; i < 32; i++) {
    const mid = (lo + hi) / 2, b = bounds(mid);
    if (b.x2 - b.x1 <= viewport.x2 - viewport.x1 && b.y2 - b.y1 <= viewport.y2 - viewport.y1) lo = mid;
    else hi = mid;
  }
  const b = bounds(lo);
  return {
    zoom: lo,
    pan: { x: (viewport.x1 + viewport.x2 - b.x1 - b.x2) / 2, y: viewport.y1 - b.y1 },
  };
}
