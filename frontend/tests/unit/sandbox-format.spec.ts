import { describe, expect, it } from "vitest";
import { fullTimestamp, sessionOpenByDefault, shortTimestamp } from "../../src/sandbox/format";

describe("sandbox panel format helpers", () => {
  it("shortens RFC 3339 timestamps for the summary row", () => {
    expect(shortTimestamp("2026-10-30T22:56:46.409+08:00")).toBe("10-30 22:56");
    expect(fullTimestamp("2026-10-30T22:56:46.409+08:00")).toBe("2026-10-30 22:56:46");
  });

  it("returns unparseable values unchanged", () => {
    expect(shortTimestamp("unknown")).toBe("unknown");
    expect(fullTimestamp("")).toBe("");
  });

  it("expands live sessions and collapses discarded ones by default", () => {
    expect(sessionOpenByDefault("active")).toBe(true);
    expect(sessionOpenByDefault("exported")).toBe(true);
    expect(sessionOpenByDefault("discarded")).toBe(false);
  });
});
