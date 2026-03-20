import { useEffect, useState } from "react";
import { setOnBackendRestarting } from "@/lib/graphql/urql-client";
import { getRuntimeBasePath } from "@/lib/runtime-config";

function getHealthUrl(): string {
  const basePath = getRuntimeBasePath();
  return basePath === "/" ? "/health" : `${basePath}/health`;
}

export function useBackendRestarting() {
  const [serviceRestarting, setServiceRestarting] = useState(false);

  useEffect(() => {
    setOnBackendRestarting(() => setServiceRestarting(true));
    return () => setOnBackendRestarting(null);
  }, []);

  useEffect(() => {
    if (!serviceRestarting) return;

    let timeoutId: ReturnType<typeof setTimeout>;
    const healthUrl = getHealthUrl();

    async function poll() {
      try {
        const response = await fetch(healthUrl);
        const data = await response.json();
        if (data.status === "ok") {
          setServiceRestarting(false);
          window.location.reload();
          return;
        }
      } catch {
        // Backend is still booting; keep polling.
      }
      timeoutId = setTimeout(poll, 2000);
    }

    timeoutId = setTimeout(poll, 2000);
    return () => clearTimeout(timeoutId);
  }, [serviceRestarting]);

  return {
    serviceRestarting,
    setServiceRestarting,
  };
}
