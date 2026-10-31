import { describe, expect, it } from "vitest";

import {
  DEFAULT_SIDEBAR_WIDTH,
  MIN_SIDEBAR_WIDTH,
  SIDEBAR_MIN_VIEWPORT,
  clampSidebarWidth,
  effectiveDockMode,
  maxSidebarWidth,
  parseDockMode,
  parseSidebarWidth,
} from "../../src/sandbox/layout";

describe("sandbox dock layout", () => {
  it("defaults to the sidebar and keeps known modes", () => {
    expect(parseDockMode(null)).toBe("sidebar");
    expect(parseDockMode("garbage")).toBe("sidebar");
    expect(parseDockMode("floating")).toBe("floating");
    expect(parseDockMode("sidebar")).toBe("sidebar");
  });

  it("clamps the sidebar so the map keeps its minimum width", () => {
    expect(maxSidebarWidth(1400)).toBe(620);
    expect(maxSidebarWidth(1920)).toBe(900);
    expect(maxSidebarWidth(1200)).toBe(420);
    expect(clampSidebarWidth(2000, 1200)).toBe(420);
    expect(clampSidebarWidth(10, 1400)).toBe(MIN_SIDEBAR_WIDTH);
    expect(clampSidebarWidth(Number.NaN, 1400)).toBe(DEFAULT_SIDEBAR_WIDTH);
  });

  it("parses the stored width with a default fallback", () => {
    expect(parseSidebarWidth(null, 1400)).toBe(DEFAULT_SIDEBAR_WIDTH);
    expect(parseSidebarWidth("", 1400)).toBe(DEFAULT_SIDEBAR_WIDTH);
    expect(parseSidebarWidth("abc", 1400)).toBe(DEFAULT_SIDEBAR_WIDTH);
    expect(parseSidebarWidth("512.4", 1400)).toBe(512);
  });

  it("renders floating in a window too narrow for a sidebar, without changing the preference", () => {
    expect(effectiveDockMode("sidebar", SIDEBAR_MIN_VIEWPORT - 1)).toBe("floating");
    expect(effectiveDockMode("sidebar", SIDEBAR_MIN_VIEWPORT)).toBe("sidebar");
    expect(effectiveDockMode("floating", 2000)).toBe("floating");
  });
});
