import type { SettingsSection, ContentSettingsSection, ViewId } from "@/components/root/types";

export type HomePageRouteState = {
  initialView?: ViewId;
  initialSettingsSection?: SettingsSection;
  initialContentSection?: ContentSettingsSection;
};
