export interface StudyLaneLabel {
  id: string;
  lane: 0 | 1 | 2;
  order: number;
  height: number;
  indent?: number;
}

/** Pack measured captions, not point centres. Use spare height for breathing room. */
export function layoutStudyLanes(labels: readonly StudyLaneLabel[], width: number, height: number, { columns = [width / 3, width / 3, width / 3], minGap = 14, compact = false }: { columns?: readonly number[]; minGap?: number; compact?: boolean } = {}) {
  const positions = new Map<string, { x: number; y: number }>();
  let right = 0;
  for (const lane of [0, 1, 2] as const) {
    right += columns[lane];
    const rows = labels.filter(item => item.lane === lane).sort((a, b) => a.order - b.order || a.id.localeCompare(b.id));
    const contentHeight = rows.reduce((sum, item) => sum + item.height, 0);
    const gap = rows.length > 1 ? Math.max(minGap, (height - contentHeight) / (rows.length - 1)) : 0;
    let y = 0;
    for (const row of rows) {
      positions.set(row.id, { x: right - (compact ? 12 : 24) + Math.min(compact ? 4 : 18, row.indent ?? 0), y: y + row.height / 2 });
      y += row.height + gap;
    }
  }
  return positions;
}
