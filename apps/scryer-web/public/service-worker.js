const CACHE_VERSION = "v2";
const SHELL_CACHE = `scryer-shell-${CACHE_VERSION}`;
const ASSET_CACHE = `scryer-assets-${CACHE_VERSION}`;
const RESERVED_PREFIXES = [
  "/graphql",
  "/health",
  "/metrics",
  "/backups",
  "/images",
];
const STATIC_PATH_PREFIXES = [
  "/assets/",
  "/icons/",
  "/download-clients/",
  "/media-sites/",
  "/rating-sources/",
];
const STATIC_PATHS = new Set([
  "/",
  "/manifest.json",
  "/scryer-logo.svg",
  "/scryer-wordmark.svg",
  "/scryer-lockup-dark.webp",
  "/scryer-lockup-light.webp",
]);

function getScopeUrl() {
  return new URL(self.registration.scope);
}

function getScopePath() {
  const pathname = getScopeUrl().pathname;
  return pathname.endsWith("/") ? pathname : `${pathname}/`;
}

function getAppRelativePath(url) {
  const scopePath = getScopePath();
  if (!url.pathname.startsWith(scopePath)) {
    return null;
  }

  if (scopePath === "/") {
    return url.pathname || "/";
  }

  const suffix = url.pathname.slice(scopePath.length);
  return suffix ? `/${suffix}` : "/";
}

function resolveScopeUrl(path) {
  return new URL(path, getScopeUrl()).toString();
}

function isReservedPath(relativePath) {
  return RESERVED_PREFIXES.some((prefix) => relativePath === prefix || relativePath.startsWith(`${prefix}/`));
}

function isStaticAssetPath(relativePath) {
  if (STATIC_PATHS.has(relativePath)) {
    return true;
  }

  if (STATIC_PATH_PREFIXES.some((prefix) => relativePath.startsWith(prefix))) {
    return true;
  }

  if (relativePath.endsWith(".gz")) {
    return false;
  }

  return /\.(?:css|js|mjs|woff2?|png|webp|svg|jpg|jpeg|gif|ico|txt|xml)$/i.test(relativePath);
}

function isImmutableBuildAssetPath(relativePath) {
  return relativePath.startsWith("/assets/");
}

function isCacheableResponse(response) {
  return response && response.ok && response.type !== "error";
}

function isHtmlResponse(response) {
  return isCacheableResponse(response) && (response.headers.get("content-type") || "").includes("text/html");
}

async function putIfFresh(cacheName, requestInfo, requestInit) {
  try {
    const response = await fetch(requestInfo, requestInit);
    if (!isCacheableResponse(response)) {
      return null;
    }
    const cache = await caches.open(cacheName);
    await cache.put(requestInfo, response.clone());
    return response;
  } catch {
    return null;
  }
}

async function precacheShell() {
  const shellUrl = resolveScopeUrl("./");
  const iconUrls = [
    resolveScopeUrl("./scryer-logo.svg"),
    resolveScopeUrl("./scryer-wordmark.svg"),
    resolveScopeUrl("./scryer-lockup-dark.webp"),
    resolveScopeUrl("./scryer-lockup-light.webp"),
    resolveScopeUrl("./manifest.json"),
  ];

  await putIfFresh(SHELL_CACHE, shellUrl, { cache: "no-store" });

  await Promise.all(
    iconUrls.map((url) =>
      putIfFresh(ASSET_CACHE, url, { cache: "no-store" }).catch(() => null),
    ),
  );
}

async function cleanupCaches() {
  const validCacheNames = new Set([SHELL_CACHE, ASSET_CACHE]);
  const cacheNames = await caches.keys();
  await Promise.all(
    cacheNames
      .filter((name) => name.startsWith("scryer-") && !validCacheNames.has(name))
      .map((name) => caches.delete(name)),
  );
}

async function handleNavigation(request) {
  const shellUrl = resolveScopeUrl("./");

  try {
    const response = await fetch(request);
    if (isHtmlResponse(response)) {
      const cache = await caches.open(SHELL_CACHE);
      await cache.put(shellUrl, response.clone());
    }
    return response;
  } catch {
    const cache = await caches.open(SHELL_CACHE);
    const cachedShell = await cache.match(shellUrl);
    if (cachedShell) {
      return cachedShell;
    }

    return new Response("Offline", {
      status: 503,
      statusText: "Offline",
      headers: {
        "Content-Type": "text/plain; charset=utf-8",
      },
    });
  }
}

// A network error on a build chunk (a dropped connection, a device waking up)
// is retried before the page sees it; a failed import otherwise breaks the
// route. HTTP errors are real answers and are returned as they are.
const NETWORK_RETRY_DELAYS_MS = [300, 1000, 3000];

function delay(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

async function fetchWithRetry(request) {
  for (let attempt = 0; ; attempt += 1) {
    try {
      return await fetch(request.clone());
    } catch (error) {
      if (attempt >= NETWORK_RETRY_DELAYS_MS.length) {
        throw error;
      }
      await delay(NETWORK_RETRY_DELAYS_MS[attempt]);
    }
  }
}

async function handleStaticRequest(request, event, relativePath) {
  const cache = await caches.open(ASSET_CACHE);
  const immutable = isImmutableBuildAssetPath(relativePath);
  // A build asset's name carries its content hash, so one cached copy serves
  // every request for it. Matching on Vary would miss module-script requests,
  // which carry headers the background download did not.
  const cachedResponse = await cache.match(request, { ignoreVary: immutable });

  if (cachedResponse && immutable) {
    return cachedResponse;
  }

  const networkPromise = (immutable ? fetchWithRetry(request) : fetch(request))
    .then(async (response) => {
      if (isCacheableResponse(response)) {
        await cache.put(request, response.clone());
      }
      return response;
    })
    .catch(() => null);

  if (cachedResponse) {
    event.waitUntil(networkPromise);
    return cachedResponse;
  }

  const networkResponse = await networkPromise;
  if (networkResponse) {
    return networkResponse;
  }

  return Response.error();
}

// Background download of the current build's chunks, listed by the build in
// sw-precache.json, so a lazy route opened later loads from the cache when the
// network is gone. Cached files that belong to no current build are dropped
// once they are a day old; an open tab from the previous build can still use
// them until then.
const PRECACHE_MANIFEST_PATH = "./sw-precache.json";
const PRECACHE_CONCURRENCY = 2;
const STALE_ASSET_GRACE_MS = 24 * 60 * 60 * 1000;
let precacheInFlight = null;

function saveDataRequested() {
  return Boolean(self.navigator?.connection?.saveData);
}

async function readPrecacheManifest() {
  try {
    const response = await fetch(resolveScopeUrl(PRECACHE_MANIFEST_PATH), {
      cache: "no-store",
    });
    if (!response.ok) {
      return null;
    }
    const manifest = await response.json();
    if (
      !manifest ||
      !Array.isArray(manifest.precache) ||
      !Array.isArray(manifest.files)
    ) {
      return null;
    }
    return manifest;
  } catch {
    return null;
  }
}

async function pruneStaleBuildAssets(cache, currentFiles) {
  const current = new Set(currentFiles.map((file) => resolveScopeUrl(`./${file}`)));
  const now = Date.now();
  for (const request of await cache.keys()) {
    const relativePath = getAppRelativePath(new URL(request.url));
    if (
      relativePath === null ||
      !isImmutableBuildAssetPath(relativePath) ||
      current.has(request.url)
    ) {
      continue;
    }
    const response = await cache.match(request);
    const cachedAt = Date.parse(response?.headers.get("date") ?? "");
    if (!Number.isFinite(cachedAt) || cachedAt + STALE_ASSET_GRACE_MS < now) {
      await cache.delete(request);
    }
  }
}

async function precacheBuildAssets() {
  const manifest = await readPrecacheManifest();
  if (!manifest) {
    return;
  }
  const cache = await caches.open(ASSET_CACHE);
  await pruneStaleBuildAssets(cache, manifest.files);
  if (saveDataRequested()) {
    return;
  }

  const missing = [];
  for (const file of manifest.precache) {
    if (typeof file !== "string" || !file.startsWith("assets/")) {
      continue;
    }
    const url = resolveScopeUrl(`./${file}`);
    if (!(await cache.match(url, { ignoreVary: true }))) {
      missing.push(url);
    }
  }

  const worker = async () => {
    for (let url = missing.shift(); url; url = missing.shift()) {
      try {
        const response = await fetch(url);
        if (isCacheableResponse(response)) {
          await cache.put(url, response);
        }
      } catch {
        // The next page load tries again.
      }
    }
  };
  await Promise.all(
    Array.from({ length: PRECACHE_CONCURRENCY }, () => worker()),
  );
}

function schedulePrecache() {
  if (!precacheInFlight) {
    precacheInFlight = precacheBuildAssets().finally(() => {
      precacheInFlight = null;
    });
  }
  return precacheInFlight;
}

self.addEventListener("message", (event) => {
  if (event.data?.type === "scryer:precache-assets") {
    event.waitUntil(schedulePrecache());
  }
});

self.addEventListener("install", (event) => {
  event.waitUntil(precacheShell());
});

self.addEventListener("activate", (event) => {
  event.waitUntil(
    (async () => {
      await cleanupCaches();
      await self.clients.claim();
    })(),
  );
});

self.addEventListener("fetch", (event) => {
  const { request } = event;
  if (request.method !== "GET") {
    return;
  }

  const url = new URL(request.url);
  if (url.origin !== self.location.origin) {
    return;
  }

  const relativePath = getAppRelativePath(url);
  if (relativePath === null || isReservedPath(relativePath) || relativePath === "/service-worker.js") {
    return;
  }

  if (request.mode === "navigate") {
    event.respondWith(handleNavigation(request));
    return;
  }

  if (isStaticAssetPath(relativePath)) {
    event.respondWith(handleStaticRequest(request, event, relativePath));
  }
});
