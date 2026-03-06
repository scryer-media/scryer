
import { memo, useCallback, useEffect, useState } from "react";
import { useClient } from "urql";
import { SystemView } from "@/components/views/system-view";
import { systemHealthQuery } from "@/lib/graphql/queries";
import type { SystemHealth } from "@/components/root/types";
import { useTranslate } from "@/lib/context/translate-context";
import { useGlobalStatus } from "@/lib/context/global-status-context";

export const SystemContainer = memo(function SystemContainer() {
  const setGlobalStatus = useGlobalStatus();
  const t = useTranslate();
  const client = useClient();
  const [systemHealth, setSystemHealth] = useState<SystemHealth | null>(null);
  const [systemLoading, setSystemLoading] = useState(false);

  const refreshSystem = useCallback(async () => {
    setSystemLoading(true);
    try {
      const { data, error } = await client.query(systemHealthQuery, {}).toPromise();
      if (error) throw error;
      setSystemHealth(data?.systemHealth ?? null);
      setGlobalStatus(data?.systemHealth?.serviceReady ? t("system.loaded") : t("system.notReady"));
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToLoad"));
    } finally {
      setSystemLoading(false);
    }
  }, [client, setGlobalStatus, t]);

  useEffect(() => {
    void refreshSystem();
  }, [refreshSystem]);

  return (
    <SystemView
      state={{
        systemHealth,
        systemLoading,
        refreshSystem,
      }}
    />
  );
});
