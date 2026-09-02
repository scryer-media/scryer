const BASE_PATH_PLACEHOLDER = "__SCRYER_BASE_PATH__";
const GRAPHQL_URL_PLACEHOLDER = "__SCRYER_GRAPHQL_URL__";

type RuntimeConfig = {
  basePath?: string;
  graphqlUrl?: string;
};

declare global {
  interface Window {
    __SCRYER_RUNTIME_CONFIG__?: RuntimeConfig;
  }
}

function normalizeOptionalValue(value: string | undefined | null, placeholder: string): string | null {
  const trimmed = value?.trim();
  if (!trimmed || trimmed === placeholder) {
    return null;
  }
  return trimmed;
}

function normalizeBasePath(value: string | undefined | null): string {
  const trimmed = value?.trim() ?? "";
  if (!trimmed || trimmed === "/") {
    return "/";
  }

  const segments = trimmed
    .replace(/\\/g, "/")
    .split("/")
    .filter((segment) => segment.length > 0);

  return segments.length === 0 ? "/" : `/${segments.join("/")}`;
}

function defaultGraphqlUrl(basePath: string): string {
  return basePath === "/" ? "/graphql" : `${basePath}/graphql`;
}

function readRuntimeConfig(): { basePath: string; graphqlUrl: string } {
  const runtime = typeof window !== "undefined" ? window.__SCRYER_RUNTIME_CONFIG__ : undefined;
  const basePath = normalizeBasePath(
    normalizeOptionalValue(runtime?.basePath, BASE_PATH_PLACEHOLDER) ??
      import.meta.env.SCRYER_BASE_PATH,
  );
  const graphqlUrl =
    normalizeOptionalValue(runtime?.graphqlUrl, GRAPHQL_URL_PLACEHOLDER) ??
    import.meta.env.SCRYER_GRAPHQL_URL ??
    defaultGraphqlUrl(basePath);

  return { basePath, graphqlUrl };
}

const runtimeConfig = readRuntimeConfig();

export function getRuntimeBasePath(): string {
  return runtimeConfig.basePath;
}

export function getRuntimeGraphqlUrl(): string {
  return runtimeConfig.graphqlUrl;
}

/** An app-relative path with the deployment's base path in front of it. */
export function buildAppUrl(path: string): string {
  const basePath = getRuntimeBasePath();
  return basePath === "/" ? path : `${basePath}${path}`;
}

export function getRuntimeBackendUrl(path: string): string {
  const suffix = path.startsWith("/") ? path : `/${path}`;
  if (typeof window === "undefined") {
    const basePath = runtimeConfig.basePath === "/" ? "" : runtimeConfig.basePath;
    return `${basePath}${suffix}`;
  }

  const url = new URL(runtimeConfig.graphqlUrl, window.location.origin);
  const apiBasePath = url.pathname.replace(/\/graphql\/?$/, "").replace(/\/+$/, "");
  url.pathname = `${apiBasePath}${suffix}`;
  url.search = "";
  url.hash = "";
  return url.toString();
}
