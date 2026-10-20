import type { Page } from "@playwright/test";

/**
 * Real on-screen geometry of the desktop ink board, read from the settled
 * canvas' actual 2D transform (not from app state flags).
 */
export interface BoardGeometry {
  board: { left: number; top: number; width: number; height: number };
  paper: { left: number; top: number; width: number; height: number };
  scale: number;
  offsetX: number;
  offsetY: number;
  /** Page coordinate at the centre of the visible board. */
  centerWorld: { x: number; y: number };
}

export async function boardGeometry(page: Page): Promise<BoardGeometry> {
  return page.evaluate(() => {
    const canvas = document.querySelector<HTMLCanvasElement>("#desktop-ink-board .ink-settled")!;
    const rect = canvas.getBoundingClientRect();
    const dpr = Math.min(window.devicePixelRatio || 1, 2); // same cap as InkCanvas
    const t = canvas.getContext("2d")!.getTransform();
    const scale = t.a / dpr, offsetX = t.e / dpr, offsetY = t.f / dpr;
    const harness = (window as unknown as { appHarness?: { storedDocuments: Map<string, { pages: Array<{ width: number; height: number }> }> }; inkDesktopHarness?: { storedDocuments: Map<string, { pages: Array<{ width: number; height: number }> }> } });
    const docs = (harness.appHarness ?? harness.inkDesktopHarness)!.storedDocuments;
    const pageSize = [...docs.values()][0].pages[0];
    return {
      board: { left: rect.left, top: rect.top, width: rect.width, height: rect.height },
      paper: { left: rect.left + offsetX, top: rect.top + offsetY, width: pageSize.width * scale, height: pageSize.height * scale },
      scale, offsetX, offsetY,
      centerWorld: { x: (rect.width / 2 - offsetX) / scale, y: (rect.height / 2 - offsetY) / scale },
    };
  });
}

/** Paper fully inside the board and centred (what 整页 / fit produces). */
export function isFittedCentered(g: BoardGeometry, tolerance = 1.5): boolean {
  const paperCx = g.paper.left + g.paper.width / 2, boardCx = g.board.left + g.board.width / 2;
  const paperCy = g.paper.top + g.paper.height / 2, boardCy = g.board.top + g.board.height / 2;
  const inside = g.paper.left >= g.board.left - tolerance && g.paper.left + g.paper.width <= g.board.left + g.board.width + tolerance
    && g.paper.top >= g.board.top - tolerance && g.paper.top + g.paper.height <= g.board.top + g.board.height + tolerance;
  const fillsOneAxis = Math.abs(g.paper.width - (g.board.width - 32)) < tolerance || Math.abs(g.paper.height - (g.board.height - 32)) < tolerance;
  return inside && fillsOneAxis && Math.abs(paperCx - boardCx) < tolerance && Math.abs(paperCy - boardCy) < tolerance;
}

/** Wait until the board geometry stops changing (ResizeObserver + rAF settled). */
export async function settledGeometry(page: Page): Promise<BoardGeometry> {
  let last = "";
  for (let i = 0; i < 20; i++) {
    const g = await boardGeometry(page);
    const key = JSON.stringify([g.board, g.paper].map(r => Object.values(r).map(v => Math.round(v * 10))));
    if (key === last) return g;
    last = key;
    await page.waitForTimeout(50);
  }
  return boardGeometry(page);
}
