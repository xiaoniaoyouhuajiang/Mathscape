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
