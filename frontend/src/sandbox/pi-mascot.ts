/**
 * Pi-Automaton SVG mascot for the collapsed sandbox dialogue floating button.
 *
 * All three Mathscape themes (`obsidian`, `parchment`, `slate`) share the same
 * mathematical character silhouette — a calligraphic Greek pi crown-brim,
 * compact optical visor head with side caliper transponders, and twin sculpted
 * pi-legs (left splayed foot + right hooked foot) — while each theme renders
 * its own bespoke surface palette and instrument details via CSS `[data-theme]`.
 */

const SVG_NS = "http://www.w3.org/2000/svg";
let instanceSeq = 0;

interface ThemeVariantPalette {
  key: "obsidian" | "parchment" | "slate";
  bgInner: string;
  bgMid: string;
  bgOuter: string;
  rimStroke: string;
  rimOpacity: string;
  innerRingStroke: string;
  innerRingOpacity: string;
  piTop: string;
  piMid: string;
  piBottom: string;
  piStroke: string;
  piHighlight: string;
  headTop: string;
  headBottom: string;
  headStroke: string;
  headBevel: string;
  earFill: string;
  earRivet: string;
  earTick: string;
  visorFill: string;
  visorStroke: string;
  eyeFill: string;
  eyeGlow: string;
  smileStroke: string;
  sparkFill: string;
  sparkHalo: string;
  shadowFill: string;
  shadowOpacity: string;
  ambientMarkup: string;
}

function buildVariantGroup(uid: number, p: ThemeVariantPalette): string {
  const bgId = `ms-pibot-bg-${p.key}-${uid}`;
  const piId = `ms-pibot-pi-${p.key}-${uid}`;
  const headId = `ms-pibot-head-${p.key}-${uid}`;
  const flareId = `ms-pibot-flare-${p.key}-${uid}`;

  // Calligraphic pi architrave bar (tapered left downward curl + upturned right serif)
  // Overshoots the compact head cleanly on both left and right like a true mortarboard visor brim.
  const piBrimPath =
    "M 11.4 22.0 C 13.0 16.8, 16.8 15.2, 22.4 15.2 L 45.0 15.2 " +
    "C 48.0 15.2, 50.3 14.4, 52.0 13.0 C 52.7 12.5, 53.5 13.2, 53.0 14.2 " +
    "C 51.2 17.8, 48.0 19.5, 43.6 19.5 L 20.4 19.5 " +
    "C 17.1 19.5, 14.8 20.4, 13.1 22.6 C 12.4 23.4, 11.1 22.8, 11.4 22.0 Z";

  // Left pi leg (originates cleanly behind the lower head bezel and splays gently outward-left)
  const piLeftLegPath =
    "M 24.4 36.2 L 28.5 36.2 C 28.2 42.4, 26.8 47.9, 24.1 52.1 " +
    "C 23.3 53.3, 21.4 53.4, 20.9 52.1 C 20.5 51.1, 21.1 50.0, 21.8 48.8 " +
    "C 23.6 45.8, 24.3 41.6, 24.4 36.2 Z";

  // Right pi leg (originates behind the lower head bezel and curves into pi's signature J-hook foot)
  const piRightLegPath =
    "M 35.5 36.2 L 39.6 36.2 L 39.6 47.1 C 39.6 48.9, 40.4 49.7, 41.7 49.7 " +
    "C 42.7 49.7, 43.5 49.0, 44.2 48.0 C 44.8 47.2, 46.0 47.7, 45.6 48.9 " +
    "C 44.6 51.5, 42.6 53.1, 40.1 53.1 C 37.2 53.1, 35.5 51.1, 35.5 47.5 Z";

  // Four-point compass star antenna spark
  const starPath =
    "M 32 5.2 L 33.3 8.5 L 36.6 9.8 L 33.3 11.1 L 32 14.4 L 30.7 11.1 L 27.4 9.8 L 30.7 8.5 Z";

  return `
    <defs>
      <radialGradient id="${bgId}" cx="36%" cy="26%" r="72%">
        <stop offset="0%" stop-color="${p.bgInner}" />
        <stop offset="58%" stop-color="${p.bgMid}" />
        <stop offset="100%" stop-color="${p.bgOuter}" />
      </radialGradient>
      <linearGradient id="${piId}" x1="18%" y1="12%" x2="82%" y2="92%">
        <stop offset="0%" stop-color="${p.piTop}" />
        <stop offset="52%" stop-color="${p.piMid}" />
        <stop offset="100%" stop-color="${p.piBottom}" />
      </linearGradient>
      <linearGradient id="${headId}" x1="50%" y1="0%" x2="50%" y2="100%">
        <stop offset="0%" stop-color="${p.headTop}" />
        <stop offset="100%" stop-color="${p.headBottom}" />
      </linearGradient>
      <radialGradient id="${flareId}" cx="50%" cy="50%" r="50%">
        <stop offset="0%" stop-color="${p.sparkHalo}" stop-opacity="0.76" />
        <stop offset="100%" stop-color="${p.sparkHalo}" stop-opacity="0" />
      </radialGradient>
    </defs>
    <g class="pi-bot-variant pi-bot-variant--${p.key}">
      <circle cx="32" cy="32" r="30.5" fill="url(#${bgId})" stroke="${p.rimStroke}" stroke-opacity="${p.rimOpacity}" stroke-width="1.35" />
      <circle cx="32" cy="32" r="27.2" fill="none" stroke="${p.innerRingStroke}" stroke-opacity="${p.innerRingOpacity}" stroke-width="0.85" />
      ${p.ambientMarkup}

      <ellipse cx="32" cy="55.2" rx="11.2" ry="1.8" fill="${p.shadowFill}" fill-opacity="${p.shadowOpacity}" />

      <g class="pi-bot-character">
        <!-- Antenna mast & star spark -->
        <line x1="32" y1="11.6" x2="32" y2="15.6" stroke="${p.piMid}" stroke-width="1.65" stroke-linecap="round" />
        <circle class="pi-bot-spark-halo" cx="32" cy="9.8" r="6.5" fill="url(#${flareId})" />
        <path class="pi-bot-spark" d="${starPath}" fill="${p.sparkFill}" />

        <!-- Twin sculpted pi legs tucked cleanly behind the head chassis -->
        <path d="${piLeftLegPath}" fill="url(#${piId})" stroke="${p.piStroke}" stroke-width="0.65" stroke-linejoin="round" />
        <path d="${piRightLegPath}" fill="url(#${piId})" stroke="${p.piStroke}" stroke-width="0.65" stroke-linejoin="round" />
        <path d="M 24.7 40.3 H 28.0 M 35.9 40.3 H 39.2" stroke="${p.piHighlight}" stroke-opacity="0.45" stroke-width="0.8" stroke-linecap="round" />

        <!-- Side caliper ear transponders with precision rivets -->
        <rect x="14.2" y="25.2" width="3.6" height="8.4" rx="1.8" fill="${p.earFill}" stroke="${p.headStroke}" stroke-width="0.9" />
        <rect x="46.2" y="25.2" width="3.6" height="8.4" rx="1.8" fill="${p.earFill}" stroke="${p.headStroke}" stroke-width="0.9" />
        <circle cx="16.0" cy="29.4" r="0.7" fill="${p.earRivet}" />
        <circle cx="48.0" cy="29.4" r="0.7" fill="${p.earRivet}" />
        <path d="M 12.1 29.4 H 14.2 M 49.8 29.4 H 51.9" stroke="${p.earTick}" stroke-width="1.25" stroke-linecap="round" />

        <!-- Compact instrument head tucked neatly beneath the pi crown-brim -->
        <rect x="17.5" y="18.8" width="29.0" height="19.4" rx="7.2" fill="url(#${headId})" stroke="${p.headStroke}" stroke-width="1.15" />
        <path d="M 20.5 20.4 H 43.5" stroke="${p.headBevel}" stroke-opacity="0.42" stroke-width="0.85" stroke-linecap="round" />

        <!-- Recessed optical visor -->
        <rect x="20.4" y="21.9" width="23.2" height="13.2" rx="4.8" fill="${p.visorFill}" stroke="${p.visorStroke}" stroke-width="0.9" />
        <path d="M 23.0 23.3 H 31.5" stroke="#ffffff" stroke-opacity="0.12" stroke-width="0.8" stroke-linecap="round" />

        <!-- Luminous eyes & friendly expression -->
        <g class="pi-bot-eyes">
          <rect x="24.4" y="25.1" width="4.1" height="5.5" rx="2.05" fill="${p.eyeFill}" />
          <rect x="35.5" y="25.1" width="4.1" height="5.5" rx="2.05" fill="${p.eyeFill}" />
          <circle cx="27.2" cy="26.6" r="0.92" fill="#ffffff" fill-opacity="0.94" />
          <circle cx="38.3" cy="26.6" r="0.92" fill="#ffffff" fill-opacity="0.94" />
          <circle cx="25.6" cy="29.4" r="0.48" fill="${p.eyeGlow}" fill-opacity="0.78" />
          <circle cx="36.7" cy="29.4" r="0.48" fill="${p.eyeGlow}" fill-opacity="0.78" />
          <path d="M 30.1 31.9 Q 32 33.5 33.9 31.9" fill="none" stroke="${p.smileStroke}" stroke-width="1.35" stroke-linecap="round" />
        </g>

        <!-- Calligraphic pi crown-brim resting cleanly across the brow -->
        <path d="${piBrimPath}" fill="url(#${piId})" stroke="${p.piStroke}" stroke-width="0.7" stroke-linejoin="round" />
        <path d="M 19.0 16.6 H 43.0" stroke="${p.piHighlight}" stroke-opacity="0.45" stroke-width="0.75" stroke-linecap="round" />
      </g>
    </g>
  `;
}

const VARIANTS: readonly ThemeVariantPalette[] = [
  {
    key: "obsidian",
    bgInner: "#162440",
    bgMid: "#0a101e",
    bgOuter: "#050811",
    rimStroke: "#8ec0ff",
    rimOpacity: "0.46",
    innerRingStroke: "#8ec0ff",
    innerRingOpacity: "0.18",
    piTop: "#ffe8b0",
    piMid: "#eab665",
    piBottom: "#b97c28",
    piStroke: "#fff2ce",
    piHighlight: "#fffdf5",
    headTop: "#1b2c4b",
    headBottom: "#0e172a",
    headStroke: "#7fb4ff",
    headBevel: "#dceaff",
    earFill: "#15233c",
    earRivet: "#eab665",
    earTick: "#eab665",
    visorFill: "#050811",
    visorStroke: "#2c436b",
    eyeFill: "#98cbff",
    eyeGlow: "#dceaff",
    smileStroke: "#eab665",
    sparkFill: "#ffe39f",
    sparkHalo: "#ffd678",
    shadowFill: "#000000",
    shadowOpacity: "0.55",
    ambientMarkup: `
      <polyline points="11.8,21.5 15.2,14.8 21.2,12.2" fill="none" stroke="#8ec0ff" stroke-opacity="0.34" stroke-width="0.85" stroke-linecap="round" stroke-linejoin="round" />
      <circle cx="11.8" cy="21.5" r="0.9" fill="#dceaff" fill-opacity="0.82" />
      <circle cx="15.2" cy="14.8" r="1.05" fill="#ffe29e" fill-opacity="0.88" />
      <circle cx="21.2" cy="12.2" r="0.8" fill="#8ec0ff" fill-opacity="0.75" />
      <circle cx="51.6" cy="20.2" r="1.05" fill="#ffe29e" fill-opacity="0.82" />
      <circle cx="49.5" cy="46.2" r="0.85" fill="#8ec0ff" fill-opacity="0.68" />
    `,
  },
  {
    key: "parchment",
    bgInner: "#fdfcf8",
    bgMid: "#f3ede0",
    bgOuter: "#e3dac6",
    rimStroke: "#b5791f",
    rimOpacity: "0.64",
    innerRingStroke: "#26538d",
    innerRingOpacity: "0.24",
    piTop: "#3368ad",
    piMid: "#234b80",
    piBottom: "#17345b",
    piStroke: "#b5791f",
    piHighlight: "#9ec5f8",
    headTop: "#fbf8f0",
    headBottom: "#e7dfd0",
    headStroke: "#9c6b1e",
    headBevel: "#ffffff",
    earFill: "#ede5d4",
    earRivet: "#b5791f",
    earTick: "#b5791f",
    visorFill: "#182638",
    visorStroke: "#b5791f",
    eyeFill: "#ffe094",
    eyeGlow: "#fffdf7",
    smileStroke: "#f3c567",
    sparkFill: "#c2811f",
    sparkHalo: "#e5a93c",
    shadowFill: "#5c4928",
    shadowOpacity: "0.22",
    ambientMarkup: `
      <path d="M 32 4.8 V 6.7 M 32 57.3 V 59.2 M 4.8 32 H 6.7 M 57.3 32 H 59.2" stroke="#b5791f" stroke-opacity="0.48" stroke-width="1.05" stroke-linecap="round" />
      <path d="M 18.4 8.5 L 19.3 10.0 M 45.6 8.5 L 44.7 10.0 M 8.5 18.4 L 10.0 19.3 M 55.5 18.4 L 54.0 19.3 M 8.5 45.6 L 10.0 44.7 M 55.5 45.6 L 54.0 44.7 M 18.4 55.5 L 19.3 54.0 M 45.6 55.5 L 44.7 54.0" stroke="#26538d" stroke-opacity="0.28" stroke-width="0.85" stroke-linecap="round" />
    `,
  },
  {
    key: "slate",
    bgInner: "#1d303d",
    bgMid: "#111c24",
    bgOuter: "#091015",
    rimStroke: "#66bdb0",
    rimOpacity: "0.52",
    innerRingStroke: "#e0be74",
    innerRingOpacity: "0.20",
    piTop: "#9beadd",
    piMid: "#5bb8aa",
    piBottom: "#357d72",
    piStroke: "#d8fbf5",
    piHighlight: "#eafffc",
    headTop: "#213543",
    headBottom: "#132029",
    headStroke: "#66bdb0",
    headBevel: "#d4f7f1",
    earFill: "#192a35",
    earRivet: "#e0be74",
    earTick: "#e0be74",
    visorFill: "#071015",
    visorStroke: "#355963",
    eyeFill: "#a8f3e6",
    eyeGlow: "#e6fffb",
    smileStroke: "#e0be74",
    sparkFill: "#e8ca84",
    sparkHalo: "#66bdb0",
    shadowFill: "#000000",
    shadowOpacity: "0.50",
    ambientMarkup: `
      <path d="M 10.4 31.0 A 21.6 21.6 0 0 1 16.4 17.0" fill="none" stroke="#66bdb0" stroke-opacity="0.34" stroke-width="1.0" stroke-linecap="round" />
      <path d="M 53.6 33.0 A 21.6 21.6 0 0 1 47.6 47.0" fill="none" stroke="#66bdb0" stroke-opacity="0.34" stroke-width="1.0" stroke-linecap="round" />
      <path d="M 13.4 34.0 A 18.8 18.8 0 0 1 17.6 21.8" fill="none" stroke="#e0be74" stroke-opacity="0.22" stroke-width="0.8" stroke-linecap="round" />
      <circle cx="14.2" cy="44.8" r="0.95" fill="#e0be74" fill-opacity="0.75" />
      <circle cx="50.4" cy="18.6" r="0.95" fill="#9beadd" fill-opacity="0.78" />
    `,
  },
];

export function createPiMascotSvg(pinnedTheme?: "obsidian" | "parchment" | "slate"): SVGSVGElement {
  const uid = ++instanceSeq;
  const svg = document.createElementNS(SVG_NS, "svg");
  svg.setAttribute("class", pinnedTheme ? `pi-mascot-svg pi-mascot-svg--pinned pi-mascot-svg--${pinnedTheme}` : "pi-mascot-svg");
  svg.setAttribute("viewBox", "0 0 64 64");
  svg.setAttribute("width", "56");
  svg.setAttribute("height", "56");
  svg.setAttribute("aria-hidden", "true");
  svg.setAttribute("focusable", "false");
  const selected = pinnedTheme ? VARIANTS.filter((v) => v.key === pinnedTheme) : VARIANTS;
  svg.innerHTML = selected.map((v) => buildVariantGroup(uid, v)).join("");
  return svg;
}
