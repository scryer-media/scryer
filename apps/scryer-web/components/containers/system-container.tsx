
import { memo, type ReactNode, useCallback, useEffect, useState } from "react";
import { useClient } from "urql";
import { ChevronRight, Recycle, ScrollText, Server, TextSearch, Timer, type LucideIcon } from "lucide-react";
import { SystemLogsView, SystemView } from "@/components/views/system-view";
import { SystemAuditContainer } from "@/components/containers/system-audit-container";
import { SystemJobsContainer } from "@/components/containers/system-jobs-container";
import { SettingsRecycleBinContainer } from "@/components/containers/settings/settings-recycle-bin-container";
import { systemHealthQuery } from "@/lib/graphql/queries";
import type { SystemHealth } from "@/components/root/types";
import { useTranslate } from "@/lib/context/translate-context";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import type { LogsSection, SystemSection } from "@/components/root/types";

function SystemPageFrame({
  breadcrumbLabel,
  children,
  icon: Icon,
  maxWidthClass = "max-w-[1280px]",
  title,
}: {
  breadcrumbLabel: string;
  children: ReactNode;
  icon: LucideIcon;
  maxWidthClass?: string;
  title: string;
}) {
  const t = useTranslate();

  return (
    <div className="min-w-0 flex-1 bg-transparent">
      <div
        className={`mx-auto w-full ${maxWidthClass} px-4 py-5 sm:px-6 md:px-[30px] md:py-[26px] md:pb-[60px]`}
      >
        <div className="mb-4 flex items-center gap-1.5 text-[12.5px] text-[var(--scry-faint)]">
          <span>{t("nav.group.system")}</span>
          <ChevronRight className="h-3.5 w-3.5" />
          <span className="font-semibold text-[var(--scry-accent-text)]">
            {breadcrumbLabel}
          </span>
        </div>
        <div className="mb-6 flex min-w-0 items-center gap-4">
          <div className="flex h-[46px] w-[46px] shrink-0 items-center justify-center rounded-[13px] border border-[var(--scry-baccent)] bg-[linear-gradient(135deg,rgba(var(--scry-accent-rgb),0.35),rgba(123,91,255,0.22))] text-[var(--scry-accent-text)]">
            <Icon className="h-[23px] w-[23px]" />
          </div>
          <div className="min-w-0">
            <h1 className="text-[25px] font-bold tracking-normal text-[var(--scry-ink2)]">
              {title}
            </h1>
          </div>
        </div>
        {children}
      </div>
    </div>
  );
}

export const SystemContainer = memo(function SystemContainer({
  systemSection,
  scryerVersion,
}: {
  systemSection: SystemSection | LogsSection;
  scryerVersion: string | null;
}) {
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
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToLoad"), { level: "ERROR" });
    } finally {
      setSystemLoading(false);
    }
  }, [client, setGlobalStatus, t]);

  useEffect(() => {
    if (systemSection !== "overview") {
      return;
    }
    void refreshSystem();
  }, [refreshSystem, systemSection]);

  if (systemSection === "jobs") {
    return (
      <SystemPageFrame
        breadcrumbLabel={t("jobs.title")}
        icon={Timer}
        maxWidthClass="max-w-[1680px]"
        title={t("jobs.title")}
      >
        <SystemJobsContainer />
      </SystemPageFrame>
    );
  }
  if (systemSection === "logs") {
    return (
      <SystemPageFrame
        breadcrumbLabel={t("nav.serviceLogs")}
        icon={ScrollText}
        maxWidthClass="max-w-[1600px]"
        title={t("nav.serviceLogs")}
      >
        <SystemLogsView />
      </SystemPageFrame>
    );
  }
  if (systemSection === "audit") {
    return (
      <SystemPageFrame
        breadcrumbLabel={t("nav.auditLogs")}
        icon={TextSearch}
        maxWidthClass="max-w-[1520px]"
        title={t("nav.auditLogs")}
      >
        <SystemAuditContainer />
      </SystemPageFrame>
    );
  }
  if (systemSection === "recycleBin") {
    return (
      <SystemPageFrame
        breadcrumbLabel={t("settings.recycleBin")}
        icon={Recycle}
        maxWidthClass="max-w-[1520px]"
        title={t("settings.recycleBin")}
      >
        <SettingsRecycleBinContainer />
      </SystemPageFrame>
    );
  }

  const systemTitle = t("system.title");

  return (
    <SystemPageFrame breadcrumbLabel="Health" icon={Server} title={systemTitle}>
      <SystemView
        scryerVersion={scryerVersion}
        state={{
          systemHealth,
          systemLoading,
          refreshSystem,
        }}
      />
    </SystemPageFrame>
  );
});
