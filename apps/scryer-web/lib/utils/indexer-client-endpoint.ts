/**
 * The URL a Newznab or Torznab tool needs. *arr apps add their default
 * "/api" API path themselves, so it is not part of this value.
 *
 * The public URL may already carry the base path; when it has no path the
 * base path is appended. Without a public URL the browser origin is used.
 */
export function indexerClientEndpointUrl(
  protocol: "NEWZNAB" | "TORZNAB",
  publicUrl: string | null,
  basePath: string,
  browserOrigin: string,
): string {
  const base = normalizedBasePath(basePath);
  let root = `${browserOrigin}${base}`;
  if (publicUrl) {
    try {
      const url = new URL(publicUrl);
      const path = normalizedBasePath(url.pathname);
      root = `${url.origin}${path || base}`;
    } catch {
      // An invalid public URL is not used by the server either.
    }
  }
  return `${root}/${protocol.toLowerCase()}`;
}

function normalizedBasePath(path: string): string {
  const trimmed = path.replace(/\/+$/, "");
  if (!trimmed) return "";
  return trimmed.startsWith("/") ? trimmed : `/${trimmed}`;
}
