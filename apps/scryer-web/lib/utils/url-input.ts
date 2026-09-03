import type { ConfigFieldDef } from "../types/index.ts";

/**
 * What an operator typed into a host or URL field, taken apart.
 *
 * Every one of these fields is somewhere an operator pastes an address out of
 * another window: their router's admin page, a container's compose file, a
 * tracker's welcome email. Those addresses arrive with a scheme or without
 * one, with the port in the string or in the port box, with a trailing slash
 * or a path. All of it means the same server, so all of it is accepted and
 * put where it belongs.
 */
export type HostInput = {
  /** The scheme the operator actually wrote, or null when they wrote none. */
  scheme: "http" | "https" | null;
  /** Host only. An IPv6 literal keeps its brackets, as a URL needs them. */
  host: string;
  /** Port, or "" when the input carried none. */
  port: string;
  /** Path, without a trailing slash, or "" when there was none. */
  path: string;
};

/**
 * Take a typed address apart, or answer null when it is not one we should
 * touch.
 *
 * Null is deliberate rather than a best effort: a value carrying credentials
 * (`http://user:pw@host`) or a scheme that is not http(s) is left exactly as
 * typed, so nothing is silently dropped or rewritten and the operator sees
 * their own text in the error the server gives back.
 */
export function parseHostInput(raw: string): HostInput | null {
  const trimmed = raw.trim();
  if (trimmed === "") {
    return null;
  }
  const written = /^([A-Za-z][A-Za-z0-9+.-]*):\/\//.exec(trimmed);
  const scheme = written?.[1]?.toLowerCase() ?? null;
  if (scheme !== null && scheme !== "http" && scheme !== "https") {
    return null;
  }
  const candidate = scheme ? trimmed : `http://${bracketBareIpv6(trimmed)}`;
  let url: URL;
  try {
    url = new URL(candidate);
  } catch {
    return null;
  }
  if (url.username !== "" || url.password !== "") {
    return null;
  }
  if (url.hostname === "") {
    return null;
  }
  const path = `${url.pathname}${url.search}`.replace(/\/+$/, "");
  return {
    scheme: scheme === "https" ? "https" : scheme === "http" ? "http" : null,
    host: url.hostname,
    port: url.port,
    path: path === "" || path === "/" ? "" : path,
  };
}

/**
 * `fd00::1` is a host, not a host and a port: only a bracketed literal can
 * carry one, so an unbracketed value with two or more colons is the address.
 */
function bracketBareIpv6(value: string): string {
  return !value.startsWith("[") && value.split(":").length > 2
    ? `[${value}]`
    : value;
}

/**
 * The scheme to assume when the operator wrote none.
 *
 * Only ever consulted for a value with no scheme of its own, and the answer is
 * written back into the field on save, so the operator sees what was chosen
 * rather than having it hidden from them.
 *
 * A port that is not 443 means a service someone stood up themselves, which is
 * overwhelmingly plain HTTP; so does a private, loopback or link-local address,
 * and so does a bare hostname with no dot in it, which is a container or a LAN
 * name. Everything else is a name on the public internet, where HTTPS is the
 * only reasonable default.
 */
export function defaultSchemeFor(host: string, port: string): "http" | "https" {
  if (port !== "" && port !== "443") {
    return "http";
  }
  const bare = host.replace(/^\[/, "").replace(/\]$/, "").toLowerCase();
  if (
    bare === "localhost" ||
    bare.endsWith(".localhost") ||
    bare.endsWith(".local") ||
    bare.endsWith(".internal") ||
    bare.endsWith(".lan")
  ) {
    return "http";
  }
  if (!bare.includes(".") && !bare.includes(":")) {
    // A single label: a container name or a LAN host, never a public site.
    return "http";
  }
  return isPrivateAddress(bare) ? "http" : "https";
}

/** Loopback, private, carrier-grade NAT and link-local literals. */
function isPrivateAddress(host: string): boolean {
  const ipv4 = /^(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.(\d{1,3})$/.exec(host);
  if (ipv4) {
    const [first, second] = [Number(ipv4[1]), Number(ipv4[2])];
    if (first === 10 || first === 127) return true;
    if (first === 192 && second === 168) return true;
    if (first === 172 && second >= 16 && second <= 31) return true;
    if (first === 169 && second === 254) return true;
    // Carrier-grade NAT, which is also where Tailscale lives.
    if (first === 100 && second >= 64 && second <= 127) return true;
    return false;
  }
  if (host.includes(":")) {
    return (
      host === "::1" ||
      host.startsWith("fc") ||
      host.startsWith("fd") ||
      host.startsWith("fe80")
    );
  }
  return false;
}

/**
 * A typed address as it should be stored: the scheme the operator wrote, or the
 * one their address implies, and no trailing slash.
 *
 * Anything this cannot take apart is handed back trimmed and otherwise
 * untouched, so a value we do not understand still reaches the server, which
 * says what is wrong with it.
 */
export function normalizeUrlInput(raw: string): string {
  const parsed = parseHostInput(raw);
  if (!parsed) {
    return raw.trim();
  }
  const scheme = parsed.scheme ?? defaultSchemeFor(parsed.host, parsed.port);
  const port = parsed.port === "" ? "" : `:${parsed.port}`;
  return `${scheme}://${parsed.host}${port}${parsed.path}`;
}

/**
 * Normalize the connection URL an indexer provider declares.
 *
 * A provider's fields are its own, so only the one field carrying the
 * `CONNECTION_URL` role is touched; an API key or a category list is left
 * exactly as typed.
 */
export function normalizeIndexerConfigValues(
  fields: readonly ConfigFieldDef[],
  configValues: Record<string, string>,
): Record<string, string> {
  const urlKeys = fields
    .filter((field) => field.role === "CONNECTION_URL")
    .map((field) => field.key);
  let changed = false;
  const next = { ...configValues };
  for (const key of urlKeys) {
    const value = next[key];
    if (typeof value !== "string" || value.trim() === "") {
      continue;
    }
    const normalized = normalizeUrlInput(value);
    if (normalized !== value) {
      next[key] = normalized;
      changed = true;
    }
  }
  return changed ? next : configValues;
}
