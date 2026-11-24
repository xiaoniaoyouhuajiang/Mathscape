# Mathscape promotional artwork

The hero gives **Mathscape** a literal visual identity: an open mathematical atlas becomes a folded-paper mountain landscape, and a golden **M-shaped route** echoes the existing [app icon's constellation](../../apps/mathscape-desktop/icons/icon.svg). Faceted elevation contours suggest the geometry of the landscape. It is an original editorial composition, not a UI mockup or a generic labelled-node graph.

The promotional art uses a deep-blue night atlas, warm paper, and a gold summit beacon for a recognizable silhouette at thumbnail sizes. The separate product screenshot uses the owner's preferred **Warm Parchment** [desktop theme](../../frontend/src/theme.css). Cover art and actual-use imagery have different visual jobs; they need not share one background color.

The hero is a **concept illustration, not a product screenshot**. Do not use it to imply that complete proof workflows or automated verification are implemented.

| Asset | Use |
| --- | --- |
| [mathscape-hero.svg](mathscape-hero.svg) | Editable, self-contained 1600 × 900 vector master |
| [mathscape-hero.png](mathscape-hero.png) | 16:9 raster export used by the English README; reusable in posts or video title cards |
| [mathscape-hero.zh-CN.svg](mathscape-hero.zh-CN.svg) | Editable Chinese-language 1600 × 900 vector master |
| [mathscape-hero.zh-CN.png](mathscape-hero.zh-CN.png) | Chinese raster export used by the Chinese README |
| [workspace-parchment.png](workspace-parchment.png) | Unretouched 1280 × 820 native desktop capture of the two-book Fibonacci sample, in Warm Parchment |

The SVGs have no external images, scripts, or font downloads. The English version uses DejaVu Sans, DejaVu Sans Mono, and DejaVu Serif; the Chinese version adds Noto Sans CJK SC and Noto Serif CJK SC, with generic fallbacks. Install those fonts before exporting if you want matching line lengths; check labels after changing fonts. The original artwork is covered by the repository's [Apache-2.0 license](../../LICENSE).

The desktop screenshot uses synthetic sample data from the [two-book Fibonacci fixture](../../fixtures/emir/fibonacci-two-books/README.md), not a personal conversation. It was captured from the native Tauri app during the UI audit; graph labels and optional-runtime setup cards are shown as-is. Keep real screenshots separate from the conceptual artwork when adapting these assets.

## Export

After editing the SVG, export it at **1600 × 900** with an SVG-capable editor, or use the project's Playwright dependency. From the repository root, after installing frontend dependencies and Playwright Chromium:

```sh
frontend/node_modules/.bin/playwright screenshot \
  --browser chromium \
  --viewport-size "1600,900" \
  "file://$PWD/docs/assets/mathscape-hero.svg" \
  docs/assets/mathscape-hero.png
```

For the Chinese export, use `mathscape-hero.zh-CN.svg` as input and `mathscape-hero.zh-CN.png` as output. Keep both language variants and their PNG exports in sync. Preview at a typical README width as well as full size; preserve the book-to-landscape silhouette, golden M-shaped route, and experimental / illustration context when adapting the composition.
