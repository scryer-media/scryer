import { getRuntimeBasePath } from "@/lib/runtime-config";

function getServiceWorkerScope(basePath: string): string {
  return basePath === "/" ? "/" : `${basePath}/`;
}

function getServiceWorkerUrl(basePath: string): string {
  return basePath === "/" ? "/service-worker.js" : `${basePath}/service-worker.js`;
}

export function registerServiceWorker(): void {
  if (import.meta.env.DEV || typeof window === "undefined" || !("serviceWorker" in navigator)) {
    return;
  }

  const basePath = getRuntimeBasePath();
  const serviceWorkerUrl = getServiceWorkerUrl(basePath);
  const serviceWorkerScope = getServiceWorkerScope(basePath);

  const register = async () => {
    try {
      await navigator.serviceWorker.register(serviceWorkerUrl, {
        scope: serviceWorkerScope,
        updateViaCache: "none",
      });
    } catch {
      // Keep registration failure silent; installability should degrade gracefully.
    }
  };

  if (document.readyState === "complete") {
    void register();
    return;
  }

  window.addEventListener(
    "load",
    () => {
      void register();
    },
    { once: true },
  );
}
