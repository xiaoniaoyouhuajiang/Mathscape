/** Static, theme-coloured light; the taxonomy silhouette remains the hit target. */
export function nodeHalo(color: string, star = false): string {
  const rays = star
    ? `<path d="M24 5v9m0 20v9M5 24h9m20 0h9" stroke="${color}" stroke-width=".7" opacity=".45"/>`
    : "";
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="48" height="48" viewBox="0 0 48 48"><defs><radialGradient id="g"><stop stop-color="${color}" stop-opacity=".24"/><stop offset=".45" stop-color="${color}" stop-opacity=".09"/><stop offset="1" stop-color="${color}" stop-opacity="0"/></radialGradient></defs><circle cx="24" cy="24" r="24" fill="url(#g)"/>${rays}</svg>`;
  return `data:image/svg+xml;charset=utf-8,${encodeURIComponent(svg)}`;
}
