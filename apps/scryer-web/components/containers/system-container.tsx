
import { useCallback, useEffect, useState } from "react";
import { useClient } from "urql";
import { SystemView } from "@/components/views/system-view";
import { systemHealthQuery } from "@/lib/graphql/queries";
import type { SystemHealth, Translate } from "@/components/root/types";

type SystemContainerProps = {
  t: Translate;
  setGlobalStatus: (status: string) => void;
};

export function SystemContainer({
  t,
  setGlobalStatus,
}: SystemContainerProps) {
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
        t,
        systemHealth,
        systemLoading,
        refreshSystem,
      }}
    />
  );
}
