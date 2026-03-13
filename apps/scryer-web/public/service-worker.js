const CACHE_VERSION = "v1";
const SHELL_CACHE = `scryer-shell-${CACHE_VERSION}`;
const ASSET_CACHE = `scryer-assets-${CACHE_VERSION}`;
const RESERVED_PREFIXES = [
  "/graphql",
  "/graphiql",
  "/health",
  "/metrics",
  "/admin",
  "/images",
];
const STATIC_PATH_PREFIXES = [
  "/assets/",
  "/icons/",
  "/download-clients/",
  "/media-sites/",
];
const STATIC_PATHS = new Set([
  "/",
  "/manifest.json",
  "/favicon.webp",
  "/logo.svg",
  "/logo.webp",
  "/scryer-favicon.svg",
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

  return /\.(?:css|js|mjs|woff2?|png|webp|svg|jpg|jpeg|gif|ico|txt|xml)$/i.test(relativePath);
}

function isCacheableResponse(response) {
  return response && response.ok && response.type !== "error";
}

function isHtmlResponse(response) {
  return isCacheableResponse(response) && (response.headers.get("content-type") || "").includes("text/html");
}

function extractShellAssetUrls(html, shellUrl) {
  const assetPattern = /\b(?:src|href)=["']([^"'#]+)["']/gi;
  const urls = new Set();
  let match;
  while ((match = assetPattern.exec(html)) !== null) {
    const candidate = match[1];
    const resolved = new URL(candidate, shellUrl);
    const relativePath = getAppRelativePath(resolved);
    if (resolved.origin !== self.location.origin || relativePath === null) {
      continue;
    }
    if (isReservedPath(relativePath) || relativePath === "/service-worker.js") {
      continue;
    }
    if (isStaticAssetPath(relativePath)) {
      urls.add(resolved.toString());
    }
  }

  return Array.from(urls);
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
    resolveScopeUrl("./icons/apple-touch-icon.png"),
    resolveScopeUrl("./icons/icon-192.png"),
    resolveScopeUrl("./icons/icon-512.png"),
    resolveScopeUrl("./icons/icon-maskable-512.png"),
    resolveScopeUrl("./favicon-light.png"),
    resolveScopeUrl("./favicon-dark.png"),
    resolveScopeUrl("./favicon.webp"),
    resolveScopeUrl("./manifest.json"),
  ];

  const shellResponse = await putIfFresh(SHELL_CACHE, shellUrl, { cache: "no-store" });
  const discoveredAssetUrls = [];
  if (isHtmlResponse(shellResponse)) {
    discoveredAssetUrls.push(...extractShellAssetUrls(await shellResponse.clone().text(), shellUrl));
  }

  await Promise.all(
    [...iconUrls, ...discoveredAssetUrls].map((url) =>
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

async function handleStaticRequest(request, event) {
  const cache = await caches.open(ASSET_CACHE);
  const cachedResponse = await cache.match(request);
  const networkPromise = fetch(request)
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
    event.respondWith(handleStaticRequest(request, event));
  }
});
