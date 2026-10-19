/**
 * N3.0: the desktop offers only LAN addresses by default. VPN / CGNAT
 * (Tailscale), virtual bridges, link-local, public and loopback addresses are
 * kept in a collapsed "其他地址" group the user can pick from manually. The
 * desktop backend classifies by interface name and range (`url_kinds`); the
 * address-only fallback here covers responses without kinds.
 */
export type AddressKind = "lan" | "vpn" | "virtual" | "link_local" | "public" | "loopback";
export interface AddressOption { url: string; host: string; kind: AddressKind }

export const PREFERRED_HOST_KEY = "mathscape:ink-lan-preferred-host";
export const ADDRESS_KIND_LABELS: Record<AddressKind, string> = {
  lan: "局域网", vpn: "VPN / Tailscale", virtual: "虚拟网卡", link_local: "链路本地", public: "公网", loopback: "本机测试",
};
const KINDS = new Set<AddressKind>(["lan", "vpn", "virtual", "link_local", "public", "loopback"]);

export function classifyHost(host: string): AddressKind {
  if (host === "localhost") return "loopback";
  const octets = host.split(".").map(Number);
  if (octets.length !== 4 || octets.some(value => !Number.isInteger(value) || value < 0 || value > 255)) return "public";
  const [a, b] = octets;
  if (a === 127) return "loopback";
  if (a === 169 && b === 254) return "link_local";
  if (a === 100 && b >= 64 && b < 128) return "vpn";
  if (a === 10 || (a === 172 && b >= 16 && b < 32) || (a === 192 && b === 168)) return "lan";
  return "public";
}

/** Pair each URL with its kind; LAN first, the backend order kept otherwise. */
export function addressOptions(urls: readonly string[], kinds?: readonly string[]): AddressOption[] {
  const options = urls.map((url, index) => {
    const host = new URL(url).hostname;
    const given = kinds?.[index] as AddressKind | undefined;
    return { url, host, kind: given && KINDS.has(given) ? given : classifyHost(host) };
  });
  return [...options.filter(option => option.kind === "lan"), ...options.filter(option => option.kind !== "lan")];
}

/** The remembered host when it is still offered, otherwise the first LAN address. Never a non-LAN default. */
export function defaultAddress(options: readonly AddressOption[], preferredHost: string | null): AddressOption | undefined {
  return options.find(option => preferredHost !== null && option.host === preferredHost) ?? options.find(option => option.kind === "lan");
}

export function loadPreferredHost(): string | null {
  try { return localStorage.getItem(PREFERRED_HOST_KEY); } catch { return null; }
}
export function savePreferredHost(host: string): void {
  try { localStorage.setItem(PREFERRED_HOST_KEY, host); } catch { /* Optional preference. */ }
}

function label(option: AddressOption): string {
  return option.kind === "lan" ? option.host : `${option.host}（${ADDRESS_KIND_LABELS[option.kind]}）`;
}

/**
 * Fill an address `<select>`: LAN options always; other kinds only when the
 * "其他地址" toggle is on or the remembered choice is one of them. Without any
 * LAN address the select starts on a placeholder so no QR is drawn until the
 * user picks one explicitly. Returns whether non-LAN options exist.
 */
export function fillAddressSelect(select: HTMLSelectElement, options: readonly AddressOption[], showOther: boolean): { hasOther: boolean; hasLan: boolean; selected: AddressOption | undefined } {
  const previous = select.value;
  const preferred = loadPreferredHost();
  const chosen = options.find(option => option.url === previous) ?? defaultAddress(options, preferred);
  const lan = options.filter(option => option.kind === "lan"), other = options.filter(option => option.kind !== "lan");
  const expand = showOther || (chosen !== undefined && chosen.kind !== "lan");
  select.replaceChildren();
  if (!chosen) {
    const placeholder = new Option(lan.length || !other.length ? "未发现可用地址" : "未发现局域网地址，请在其他地址中手动选择", "");
    placeholder.disabled = true; select.add(placeholder);
  }
  if (lan.length) {
    const group = document.createElement("optgroup"); group.label = "局域网（推荐）";
    for (const option of lan) group.append(new Option(label(option), option.url));
    select.append(group);
  }
  if (other.length && expand) {
    const group = document.createElement("optgroup"); group.label = "其他地址"; group.dataset.otherAddresses = "";
    for (const option of other) group.append(new Option(label(option), option.url));
    select.append(group);
  }
  select.value = chosen?.url ?? "";
  return { hasOther: other.length > 0, hasLan: lan.length > 0, selected: chosen };
}
