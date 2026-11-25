/**
 * Visual appearance cards for Display Theme (`obsidian`, `parchment`, `slate`)
 * and Map Style (`constellation`, `classic`) inside Mathscape Settings.
 *
 * Each theme card renders a miniature three-pane Mathscape workspace chrome
 * (topbar, source sidebar, constellation map nodes, inspector) in that theme's
 * authentic palette alongside its dedicated Xiao-Pi SVG mascot variant.
 */
import { t, type MessageKey } from "../i18n";
import { createPiMascotSvg } from "../sandbox/pi-mascot";
import type { MapStyle } from "../shell/map-style";
import { themeLabel, type Theme } from "../shell/theme-manager";

interface MiniThemeChrome {
  theme: Theme;
  descKey: MessageKey;
  canvas: string;
  panel: string;
  panelDeep: string;
  panelRaised: string;
  border: string;
  text: string;
  textMuted: string;
  accent: string;
  gold: string;
  action: string;
  debt: string;
  dot: string;
}

const THEME_PREVIEWS: readonly MiniThemeChrome[] = [
  {
    theme: "obsidian",
    descKey: "theme.obsidian.desc",
    canvas: "#05070d",
    panel: "#0a0e17",
    panelDeep: "#070a12",
    panelRaised: "#131b2c",
    border: "#1d283d",
    text: "#edf3ff",
    textMuted: "#7486a6",
    accent: "#8ec0ff",
    gold: "#eab665",
    action: "#1c3558",
    debt: "#f2a089",
    dot: "rgba(168, 198, 245, 0.22)",
  },
  {
    theme: "parchment",
    descKey: "theme.parchment.desc",
    canvas: "#f7f5ef",
    panel: "#efece3",
    panelDeep: "#e8e4d9",
    panelRaised: "#e2ddd0",
    border: "#cbc5b8",
    text: "#1a2433",
    textMuted: "#6c788a",
    accent: "#26538d",
    gold: "#b5791f",
    action: "#234b80",
    debt: "#a13e2b",
    dot: "rgba(38, 83, 141, 0.20)",
  },
  {
    theme: "slate",
    descKey: "theme.slate.desc",
    canvas: "#0a0f14",
    panel: "#111921",
    panelDeep: "#0d141a",
    panelRaised: "#192631",
    border: "#253542",
    text: "#e6eff0",
    textMuted: "#799097",
    accent: "#66bdb0",
    gold: "#e0be74",
    action: "#1f4c47",
    debt: "#e9a18c",
    dot: "rgba(102, 189, 176, 0.22)",
  },
];

let sceneSeq = 0;

function buildThemeSceneSvg(p: MiniThemeChrome): string {
  const uid = ++sceneSeq;
  const clipId = `ms-theme-scene-clip-${p.theme}-${uid}`;
  const glowId = `ms-theme-scene-glow-${p.theme}-${uid}`;
  return `
    <svg class="display-theme-scene" viewBox="0 0 184 106" aria-hidden="true" focusable="false">
      <defs>
        <clipPath id="${clipId}">
          <rect x="0.5" y="0.5" width="183" height="105" rx="9.5" />
        </clipPath>
        <radialGradient id="${glowId}" cx="50%" cy="50%" r="50%">
          <stop offset="0%" stop-color="${p.gold}" stop-opacity="0.34" />
          <stop offset="100%" stop-color="${p.gold}" stop-opacity="0" />
        </radialGradient>
      </defs>
      <g clip-path="url(#${clipId})">
        <!-- Main map canvas -->
        <rect x="0" y="0" width="184" height="106" fill="${p.canvas}" />

        <!-- Topbar chrome -->
        <rect x="0" y="0" width="184" height="17" fill="${p.panelDeep}" />
        <line x1="0" y1="17" x2="184" y2="17" stroke="${p.border}" stroke-width="1" />
        <rect x="6" y="4" width="9" height="9" rx="2.4" fill="#060913" stroke="${p.accent}" stroke-opacity="0.4" stroke-width="0.6" />
        <circle cx="9.5" cy="7.5" r="1.3" fill="#ffe29e" />
        <rect x="18" y="6.8" width="24" height="3.4" rx="1.7" fill="${p.text}" fill-opacity="0.82" />
        <rect x="46" y="5" width="34" height="7" rx="3.5" fill="${p.panelRaised}" stroke="${p.border}" stroke-width="0.6" />
        <rect x="118" y="5" width="16" height="7" rx="2.2" fill="${p.panelRaised}" />
        <rect x="137" y="5" width="14" height="7" rx="2.2" fill="${p.panelRaised}" />
        <rect x="154" y="4.5" width="24" height="8" rx="2.6" fill="${p.action}" />

        <!-- Left source sidebar -->
        <rect x="0" y="17.5" width="28" height="88.5" fill="${p.panel}" />
        <line x1="28" y1="17.5" x2="28" y2="106" stroke="${p.border}" stroke-width="1" />
        <rect x="4" y="23" width="9" height="8" rx="1.8" fill="${p.panelRaised}" />
        <rect x="15" y="23" width="9" height="8" rx="1.8" fill="${p.panelRaised}" />
        <rect x="4" y="35" width="20" height="3" rx="1.5" fill="${p.textMuted}" fill-opacity="0.55" />
        <rect x="4" y="42" width="16" height="2.6" rx="1.3" fill="${p.textMuted}" fill-opacity="0.35" />
        <rect x="4" y="48" width="18" height="2.6" rx="1.3" fill="${p.textMuted}" fill-opacity="0.35" />
        <rect x="4" y="58" width="20" height="14" rx="2.2" fill="${p.panelRaised}" fill-opacity="0.7" />

        <!-- Right inspector panel -->
        <rect x="152" y="17.5" width="32" height="88.5" fill="${p.panel}" />
        <line x1="152" y1="17.5" x2="152" y2="106" stroke="${p.border}" stroke-width="1" />
        <rect x="156" y="23" width="18" height="3.2" rx="1.6" fill="${p.text}" fill-opacity="0.72" />
        <rect x="156" y="30" width="24" height="16" rx="2.4" fill="${p.panelRaised}" />
        <rect x="156" y="50" width="24" height="12" rx="2.4" fill="${p.panelRaised}" fill-opacity="0.75" />

        <!-- Map toolbar strip -->
        <rect x="28.5" y="17.5" width="123" height="11" fill="${p.panel}" fill-opacity="0.55" />
        <line x1="28.5" y1="28.5" x2="151.5" y2="28.5" stroke="${p.border}" stroke-opacity="0.65" stroke-width="0.7" />
        <rect x="33" y="20.2" width="14" height="5.4" rx="1.6" fill="${p.action}" fill-opacity="0.85" />
        <rect x="49" y="20.2" width="11" height="5.4" rx="1.6" fill="${p.panelRaised}" />
        <rect x="62" y="20.2" width="11" height="5.4" rx="1.6" fill="${p.panelRaised}" />

        <!-- Map dot grid -->
        <g fill="${p.dot}">
          <circle cx="40" cy="40" r="0.8" /><circle cx="58" cy="40" r="0.8" /><circle cx="76" cy="40" r="0.8" /><circle cx="94" cy="40" r="0.8" /><circle cx="112" cy="40" r="0.8" /><circle cx="130" cy="40" r="0.8" />
          <circle cx="40" cy="58" r="0.8" /><circle cx="58" cy="58" r="0.8" /><circle cx="76" cy="58" r="0.8" /><circle cx="94" cy="58" r="0.8" /><circle cx="112" cy="58" r="0.8" /><circle cx="130" cy="58" r="0.8" />
          <circle cx="40" cy="76" r="0.8" /><circle cx="58" cy="76" r="0.8" /><circle cx="76" cy="76" r="0.8" /><circle cx="94" cy="76" r="0.8" />
          <circle cx="40" cy="94" r="0.8" /><circle cx="58" cy="94" r="0.8" /><circle cx="76" cy="94" r="0.8" />
        </g>

        <!-- Mini mathematical constellation graph -->
        <line x1="46" y1="50" x2="72" y2="57" stroke="${p.gold}" stroke-opacity="0.58" stroke-width="1.2" />
        <line x1="50" y1="78" x2="72" y2="57" stroke="${p.gold}" stroke-opacity="0.58" stroke-width="1.2" />
        <line x1="46" y1="50" x2="50" y2="78" stroke="${p.accent}" stroke-opacity="0.42" stroke-width="1.0" />
        <line x1="72" y1="57" x2="95" y2="43" stroke="${p.accent}" stroke-opacity="0.48" stroke-width="1.05" />
        <line x1="72" y1="57" x2="82" y2="83" stroke="${p.debt}" stroke-opacity="0.62" stroke-width="1.0" stroke-dasharray="2.2 2" />

        <!-- Hexagon RepresentationCore node -->
        <polygon points="95,38.5 99,40.8 99,45.2 95,47.5 91,45.2 91,40.8" fill="${p.panelRaised}" stroke="${p.accent}" stroke-width="1.1" />
        <!-- Source unit nodes -->
        <circle cx="46" cy="50" r="3.4" fill="${p.accent}" />
        <circle cx="50" cy="78" r="3.4" fill="${p.accent}" />
        <!-- Shared entity gold star -->
        <circle cx="72" cy="57" r="9.5" fill="url(#${glowId})" />
        <polygon points="72,51.2 73.6,55.0 77.7,55.3 74.6,58.0 75.5,62.0 72,59.8 68.5,62.0 69.4,58.0 66.3,55.3 70.4,55.0" fill="${p.gold}" />
        <!-- Dashed Debt tag -->
        <polygon points="78,80.5 85,80.5 87.5,83 85,85.5 78,85.5" fill="${p.panel}" stroke="${p.debt}" stroke-width="0.9" stroke-dasharray="2 1.4" />

        <!-- Bottom-left mini legend pill -->
        <rect x="33" y="94" width="44" height="6.5" rx="3.2" fill="${p.panel}" fill-opacity="0.85" stroke="${p.border}" stroke-width="0.6" />
      </g>
      <rect x="0.5" y="0.5" width="183" height="105" rx="9.5" fill="none" stroke="${p.border}" stroke-width="1" />
    </svg>
  `;
}

function buildMapStyleSceneSvg(style: MapStyle): string {
  const uid = ++sceneSeq;
  const haloId = `ms-mapstyle-halo-${style}-${uid}`;
  if (style === "constellation") {
    return `
      <svg class="display-mapstyle-scene" viewBox="0 0 240 74" aria-hidden="true" focusable="false">
        <defs>
          <radialGradient id="${haloId}" cx="50%" cy="50%" r="50%">
            <stop offset="0%" stop-color="var(--ms-star-gold)" stop-opacity="0.38" />
            <stop offset="100%" stop-color="var(--ms-star-gold)" stop-opacity="0" />
          </radialGradient>
        </defs>
        <rect x="0.5" y="0.5" width="239" height="73" rx="8.5" fill="var(--ms-canvas)" stroke="var(--ms-border)" />
        <g fill="var(--ms-map-dot)">
          <circle cx="24" cy="18" r="1" /><circle cx="60" cy="18" r="1" /><circle cx="96" cy="18" r="1" /><circle cx="132" cy="18" r="1" /><circle cx="168" cy="18" r="1" /><circle cx="204" cy="18" r="1" />
          <circle cx="24" cy="38" r="1" /><circle cx="60" cy="38" r="1" /><circle cx="96" cy="38" r="1" /><circle cx="132" cy="38" r="1" /><circle cx="168" cy="38" r="1" /><circle cx="204" cy="38" r="1" />
          <circle cx="24" cy="58" r="1" /><circle cx="60" cy="58" r="1" /><circle cx="96" cy="58" r="1" /><circle cx="132" cy="58" r="1" /><circle cx="168" cy="58" r="1" /><circle cx="204" cy="58" r="1" />
        </g>
        <!-- Whisper-faint evidence edge + crisp semantic bridges -->
        <line x1="42" y1="54" x2="72" y2="38" stroke="var(--ms-text-muted)" stroke-opacity="0.14" stroke-width="1" />
        <line x1="72" y1="38" x2="122" y2="34" stroke="var(--ms-star-gold)" stroke-opacity="0.55" stroke-width="1.25" />
        <line x1="122" y1="34" x2="174" y2="26" stroke="var(--ms-accent)" stroke-opacity="0.42" stroke-width="1.15" />
        <line x1="122" y1="34" x2="168" y2="52" stroke="var(--ms-accent)" stroke-opacity="0.42" stroke-width="1.15" />
        <!-- Soft star halos + compact cores -->
        <circle cx="72" cy="38" r="9" fill="var(--ms-accent)" fill-opacity="0.14" />
        <circle cx="72" cy="38" r="3.4" fill="var(--ms-map-node-core)" stroke="var(--ms-accent)" stroke-width="1.3" />
        <circle cx="42" cy="54" r="2.3" fill="var(--ms-text-muted)" fill-opacity="0.45" />
        <circle cx="122" cy="34" r="13" fill="url(#${haloId})" />
        <polygon points="122,27.8 123.7,32.0 128.2,32.3 124.8,35.2 125.8,39.5 122,37.2 118.2,39.5 119.2,35.2 115.8,32.3 120.3,32.0" fill="var(--ms-star-gold)" />
        <circle cx="174" cy="26" r="8" fill="var(--ms-accent)" fill-opacity="0.12" />
        <circle cx="174" cy="26" r="3.2" fill="var(--ms-map-node-core)" stroke="var(--ms-accent)" stroke-width="1.2" />
        <circle cx="168" cy="52" r="3.0" fill="var(--ms-map-node-core)" stroke="var(--ms-accent)" stroke-width="1.2" />
        <!-- Sparse leader-line labels only on key landmarks -->
        <path d="M 125 27 L 133 17 H 137" fill="none" stroke="var(--ms-text-muted)" stroke-opacity="0.45" stroke-width="0.8" />
        <rect x="139" y="14.2" width="38" height="4.8" rx="2.4" fill="var(--ms-text)" fill-opacity="0.82" />
        <rect x="54" y="45" width="34" height="4.4" rx="2.2" fill="var(--ms-text-secondary)" fill-opacity="0.68" />
      </svg>
    `;
  }
  return `
    <svg class="display-mapstyle-scene" viewBox="0 0 240 74" aria-hidden="true" focusable="false">
      <rect x="0.5" y="0.5" width="239" height="73" rx="8.5" fill="var(--ms-canvas)" stroke="var(--ms-border)" />
      <!-- Full-opacity edges including provenance -->
      <line x1="42" y1="48" x2="80" y2="31" stroke="var(--ms-text-muted)" stroke-opacity="0.72" stroke-width="1.35" stroke-dasharray="3 2" />
      <line x1="80" y1="31" x2="132" y2="29" stroke="var(--ms-star-gold)" stroke-opacity="0.82" stroke-width="1.6" />
      <line x1="132" y1="29" x2="188" y2="24" stroke="var(--ms-accent)" stroke-opacity="0.78" stroke-width="1.5" />
      <line x1="132" y1="29" x2="182" y2="50" stroke="var(--ms-accent)" stroke-opacity="0.78" stroke-width="1.5" />
      <!-- Larger solid nodes without halos -->
      <circle cx="42" cy="48" r="5.5" fill="var(--ms-panel-raised)" stroke="var(--ms-text-muted)" stroke-width="1.5" />
      <circle cx="80" cy="31" r="7.2" fill="var(--ms-action)" stroke="var(--ms-accent)" stroke-width="1.7" />
      <polygon points="132,19.5 134.8,26.0 141.8,26.5 136.5,31.0 138.1,37.8 132,34.1 125.9,37.8 127.5,31.0 122.2,26.5 129.2,26.0" fill="var(--ms-star-gold)" stroke="var(--ms-text)" stroke-opacity="0.35" stroke-width="0.9" />
      <circle cx="188" cy="24" r="7.0" fill="var(--ms-action)" stroke="var(--ms-accent)" stroke-width="1.7" />
      <circle cx="182" cy="50" r="6.8" fill="var(--ms-action)" stroke="var(--ms-accent)" stroke-width="1.7" />
      <!-- Always-visible labels beneath every node -->
      <rect x="27" y="57" width="30" height="4.4" rx="2.2" fill="var(--ms-text-muted)" fill-opacity="0.78" />
      <rect x="61" y="42" width="38" height="4.8" rx="2.4" fill="var(--ms-text)" fill-opacity="0.88" />
      <rect x="111" y="41" width="42" height="4.8" rx="2.4" fill="var(--ms-text)" fill-opacity="0.90" />
      <rect x="169" y="34.5" width="38" height="4.6" rx="2.3" fill="var(--ms-text)" fill-opacity="0.85" />
      <rect x="163" y="60.5" width="38" height="4.6" rx="2.3" fill="var(--ms-text)" fill-opacity="0.85" />
    </svg>
  `;
}

export function createThemePickerCards(onSelect: (theme: Theme) => void): {
  element: HTMLElement;
  sync: (active: Theme) => void;
} {
  const group = document.createElement("div");
  group.className = "display-card-grid display-card-grid--themes";
  group.setAttribute("role", "radiogroup");
  const inputs = new Map<Theme, HTMLInputElement>();
  const cards = new Map<Theme, HTMLLabelElement>();

  for (const spec of THEME_PREVIEWS) {
    const labelText = themeLabel(spec.theme);
    const card = document.createElement("label");
    card.className = `display-choice-card display-choice-card--theme is-theme-${spec.theme}`;
    card.dataset.themeChoice = spec.theme;

    const input = document.createElement("input");
    input.type = "radio";
    input.name = "theme";
    input.value = spec.theme;
    input.className = "sr-only";
    input.setAttribute("aria-label", labelText);
    input.addEventListener("change", () => {
      if (input.checked) onSelect(spec.theme);
    });

    const stage = document.createElement("div");
    stage.className = "display-card-stage";
    stage.innerHTML = buildThemeSceneSvg(spec);

    const mascotBadge = document.createElement("div");
    mascotBadge.className = "display-card-mascot";
    const mascotSvg = createPiMascotSvg(spec.theme);
    mascotSvg.setAttribute("width", "44");
    mascotSvg.setAttribute("height", "44");
    mascotBadge.append(mascotSvg);
    stage.append(mascotBadge);

    const meta = document.createElement("div");
    meta.className = "display-card-meta";
    const titleRow = document.createElement("div");
    titleRow.className = "display-card-title-row";
    const indicator = document.createElement("span");
    indicator.className = "display-card-radio";
    indicator.setAttribute("aria-hidden", "true");
    const title = document.createElement("strong");
    title.className = "display-card-title";
    title.textContent = labelText;
    titleRow.append(indicator, title);

    const desc = document.createElement("span");
    desc.className = "display-card-desc";
    desc.textContent = t(spec.descKey);
    meta.append(titleRow, desc);

    card.append(input, stage, meta);
    group.append(card);
    inputs.set(spec.theme, input);
    cards.set(spec.theme, card);
  }

  return {
    element: group,
    sync: (active: Theme) => {
      for (const [key, input] of inputs) {
        const selected = key === active;
        input.checked = selected;
        cards.get(key)?.classList.toggle("is-selected", selected);
      }
    },
  };
}

export function createMapStylePickerCards(onSelect: (style: MapStyle) => void): {
  element: HTMLElement;
  sync: (active: MapStyle) => void;
} {
  const group = document.createElement("div");
  group.className = "display-card-grid display-card-grid--map-styles";
  group.setAttribute("role", "radiogroup");
  const styles: ReadonlyArray<{ style: MapStyle; titleKey: MessageKey; descKey: MessageKey }> = [
    { style: "constellation", titleKey: "mapStyle.constellation.title", descKey: "mapStyle.constellation.desc" },
    { style: "classic", titleKey: "mapStyle.classic.title", descKey: "mapStyle.classic.desc" },
  ];

  const inputs = new Map<MapStyle, HTMLInputElement>();
  const cards = new Map<MapStyle, HTMLLabelElement>();

  for (const spec of styles) {
    const titleText = t(spec.titleKey);
    const card = document.createElement("label");
    card.className = "display-choice-card display-choice-card--map-style";
    card.dataset.mapStyleChoice = spec.style;

    const input = document.createElement("input");
    input.type = "radio";
    input.name = "map-style";
    input.value = spec.style;
    input.className = "sr-only";
    input.setAttribute("aria-label", titleText);
    input.addEventListener("change", () => {
      if (input.checked) onSelect(spec.style);
    });

    const stage = document.createElement("div");
    stage.className = "display-card-stage display-card-stage--map-style";
    stage.innerHTML = buildMapStyleSceneSvg(spec.style);

    const meta = document.createElement("div");
    meta.className = "display-card-meta";
    const titleRow = document.createElement("div");
    titleRow.className = "display-card-title-row";
    const indicator = document.createElement("span");
    indicator.className = "display-card-radio";
    indicator.setAttribute("aria-hidden", "true");
    const title = document.createElement("strong");
    title.className = "display-card-title";
    title.textContent = titleText;
    titleRow.append(indicator, title);

    const desc = document.createElement("span");
    desc.className = "display-card-desc";
    desc.textContent = t(spec.descKey);
    meta.append(titleRow, desc);

    card.append(input, stage, meta);
    group.append(card);
    inputs.set(spec.style, input);
    cards.set(spec.style, card);
  }

  return {
    element: group,
    sync: (active: MapStyle) => {
      for (const [key, input] of inputs) {
        const selected = key === active;
        input.checked = selected;
        cards.get(key)?.classList.toggle("is-selected", selected);
      }
    },
  };
}
