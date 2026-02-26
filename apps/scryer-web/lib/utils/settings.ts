import type { AdminSetting } from "@/lib/types";
import { MEDIA_SETTING_EMPTY_VALUE } from "@/lib/constants/nzbget";

import { readJsonString } from "./serialization";

export function getSettingDisplayValue(setting?: AdminSetting | null): string {
  return readJsonString(
    setting?.effectiveValueJson ??
      setting?.valueJson ??
      setting?.defaultValueJson ??
      "",
  );
}

export function getSettingStringFromItems(
  items: AdminSetting[],
  keyName: string,
  fallback = MEDIA_SETTING_EMPTY_VALUE,
) {
  const record = items.find((item) => item.keyName === keyName);
  const parsed = getSettingDisplayValue(record).trim();
  return parsed.length ? parsed : fallback;
}
