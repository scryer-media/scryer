import type { LucideIcon } from "lucide-react";
import {
  ActivitySquare,
  History,
  MonitorCog,
  Settings,
  Users,
} from "lucide-react";
import type {
  ContentSettingsSection,
  SettingsSection,
  Translate,
  ViewId,
} from "@/components/root/types";
import { FACET_REGISTRY } from "@/lib/facets/registry";

export type RouteCommand = {
  id: string;
  label: string;
  description: string;
  keywords: string[];
  icon: LucideIcon;
  onSelect: () => void;
};

type BuildRouteCommandsArgs = {
  t: Translate;
  onNavigate: (
    nextView: ViewId,
    nextSettingsSection?: SettingsSection,
    nextContentSection?: ContentSettingsSection,
  ) => void;
};

function buildNavigate(
  onNavigate: BuildRouteCommandsArgs["onNavigate"],
  view: ViewId,
  settingsSection?: SettingsSection,
  contentSection?: ContentSettingsSection,
): () => void {
  return () => {
    onNavigate(view, settingsSection, contentSection);
  };
}

export function buildRouteCommands({ t, onNavigate }: BuildRouteCommandsArgs): RouteCommand[] {
  const mediaCommands = FACET_REGISTRY.flatMap((f) => [
    {
      id: `${f.viewId}-overview`,
      label: t(f.overviewLabelKey),
      description: t(f.navLabelKey),
      keywords: [f.viewId, f.id, "manage", "catalog", "overview", "library"],
      icon: f.icon,
      onSelect: buildNavigate(onNavigate, f.viewId as ViewId),
    },
    {
      id: `${f.viewId}-settings`,
      label: t(f.settingsLabelKey),
      description: t(f.settingsLabelKey),
      keywords: [f.viewId, f.id, "settings", "media", "paths", "folder"],
      icon: Settings,
      onSelect: buildNavigate(onNavigate, f.viewId as ViewId, undefined, "settings"),
    },
  ]);

  return [
    ...mediaCommands,
    {
      id: "activity",
      label: t("nav.activity"),
      description: t("nav.activity"),
      keywords: ["activity", "events", "log", "audit", "system"],
      icon: ActivitySquare,
      onSelect: buildNavigate(onNavigate, "activity"),
    },
    {
      id: "history",
      label: t("nav.history"),
      description: t("nav.history"),
      keywords: ["history", "imports", "import", "log", "records"],
      icon: History,
      onSelect: buildNavigate(onNavigate, "history"),
    },
    {
      id: "settings-general",
      label: `${t("nav.settings")} / ${t("settings.general")}`,
      description: t("nav.settings"),
      keywords: ["settings", "general", "preferences", "configuration", "system"],
      icon: Users,
      onSelect: buildNavigate(onNavigate, "settings", "general"),
    },
    {
      id: "settings-users",
      label: t("settings.users"),
      description: t("settings.users"),
      keywords: ["settings", "users", "accounts", "management"],
      icon: Users,
      onSelect: buildNavigate(onNavigate, "settings", "users"),
    },
    {
      id: "settings-quality-profiles",
      label: t("settings.qualityProfiles"),
      description: t("settings.qualityProfiles"),
      keywords: ["settings", "quality", "profiles", "metadata", "rules"],
      icon: Settings,
      onSelect: buildNavigate(onNavigate, "settings", "qualityProfiles"),
    },
    {
      id: "settings-download-clients",
      label: t("settings.downloadClients"),
      description: t("settings.downloadClients"),
      keywords: ["settings", "download", "clients", "indexers"],
      icon: Settings,
      onSelect: buildNavigate(onNavigate, "settings", "downloadClients"),
    },
    {
      id: "settings-indexers",
      label: t("settings.indexers"),
      description: t("settings.indexers"),
      keywords: ["settings", "indexers", "feeds", "search", "sources"],
      icon: Settings,
      onSelect: buildNavigate(onNavigate, "settings", "indexers"),
    },
    {
      id: "settings-rules",
      label: t("settings.rules"),
      description: t("settings.rules"),
      keywords: ["settings", "rules", "rego", "opa", "scoring", "custom"],
      icon: Settings,
      onSelect: buildNavigate(onNavigate, "settings", "rules"),
    },
    {
      id: "system",
      label: t("nav.system"),
      description: t("nav.system"),
      keywords: ["system", "health", "status", "database", "worker"],
      icon: MonitorCog,
      onSelect: buildNavigate(onNavigate, "system"),
    },
  ];
}
