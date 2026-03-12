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

    const healthUrl = getHealthUrl();
    const intervalId = window.setInterval(async () => {
      try {
        const response = await fetch(healthUrl);
        const data = await response.json();
        if (data.status === "ok") {
          setServiceRestarting(false);
          window.location.reload();
        }
      } catch {
        // Backend is still booting; keep polling.
      }
    }, 1000);

    return () => window.clearInterval(intervalId);
  }, [serviceRestarting]);

  return {
    serviceRestarting,
    setServiceRestarting,
  };
}
