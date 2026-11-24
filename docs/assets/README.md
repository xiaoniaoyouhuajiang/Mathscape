# Mathscape promotional artwork

The README hero fuses the existing [app icon's constellation](../../apps/mathscape-desktop/icons/icon.svg) with the app's **Warm Parchment** palette: a warm-paper canvas (`#f7f5ef`), Prussian ink blue (`#26538d`), and antique amber (`#b5791f`). A deep-blue atlas panel adds promotional contrast rather than reproducing the entire UI in one color. The paper, ink, and amber colors come from [the app's theme tokens](../../frontend/src/theme.css); the product screenshot uses the owner's preferred Warm Parchment desktop theme. A highlighted thread links a source to its relationships. The hero is a **concept illustration, not a product screenshot**. Do not use it to imply that complete proof workflows or automated verification are implemented.

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

For the Chinese export, use `mathscape-hero.zh-CN.svg` as input and `mathscape-hero.zh-CN.png` as output. Keep both language variants and their PNG exports in sync. Preview at a typical README width as well as full size; maintain the source card, graph labels, and experimental / illustration context when adapting the composition.
