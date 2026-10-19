import { describe, expect, it } from "vitest";
import { addressOptions, classifyHost, defaultAddress } from "../../src/ink/lan-addresses";

describe("LAN address options", () => {
  it("classifies hosts by range when the backend gives no kinds", () => {
    expect(classifyHost("192.168.1.20")).toBe("lan");
    expect(classifyHost("10.2.3.4")).toBe("lan");
    expect(classifyHost("172.31.0.1")).toBe("lan");
    expect(classifyHost("172.32.0.1")).toBe("public");
    expect(classifyHost("100.100.1.1")).toBe("vpn");
    expect(classifyHost("100.128.1.1")).toBe("public");
    expect(classifyHost("169.254.0.9")).toBe("link_local");
    expect(classifyHost("127.0.0.1")).toBe("loopback");
    expect(classifyHost("localhost")).toBe("loopback");
    expect(classifyHost("8.8.8.8")).toBe("public");
  });

  it("trusts backend kinds (interface names) and puts LAN first", () => {
    const options = addressOptions(
      ["https://172.17.0.1:1443/pad/", "https://100.90.0.2:1443/pad/", "https://192.168.1.20:1443/pad/", "https://127.0.0.1:1443/pad/"],
      ["virtual", "vpn", "lan", "loopback"],
    );
    expect(options.map(option => `${option.host}:${option.kind}`)).toEqual(["192.168.1.20:lan", "172.17.0.1:virtual", "100.90.0.2:vpn", "127.0.0.1:loopback"]);
    expect(addressOptions(["https://172.17.0.1:1443/pad/"], ["virtual"])[0].kind).toBe("virtual");
    expect(addressOptions(["https://10.0.0.2:1443/pad/"], ["bogus"])[0].kind).toBe("lan");
  });

  it("defaults to the remembered host if offered, else the first LAN address, never a non-LAN one", () => {
    const options = addressOptions(["https://10.0.0.9:1/", "https://192.168.1.20:1/", "https://100.90.0.2:1/", "https://127.0.0.1:1/"]);
    expect(defaultAddress(options, null)?.host).toBe("10.0.0.9");
    expect(defaultAddress(options, "192.168.1.20")?.host).toBe("192.168.1.20");
    expect(defaultAddress(options, "100.90.0.2")?.host).toBe("100.90.0.2");
    expect(defaultAddress(options, "192.168.9.9")?.host).toBe("10.0.0.9");
    expect(defaultAddress(addressOptions(["https://100.90.0.2:1/", "https://127.0.0.1:1/"]), null)).toBeUndefined();
  });
});
