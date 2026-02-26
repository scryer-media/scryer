export type AdminSetting = {
  scope: string;
  keyName: string;
  dataType: string;
  defaultValueJson: string;
  effectiveValueJson?: string | null;
  valueJson?: string | null;
  source?: string | null;
  hasOverride: boolean;
  isSensitive: boolean;
  validationJson?: string | null;
  scopeId?: string | null;
  updatedByUserId?: string | null;
  createdAt?: string | null;
  updatedAt?: string | null;
};

export type AdminSettingsResponse = {
  scope: string;
  scopeId?: string | null;
  items: AdminSetting[];
  qualityProfiles?: string | null;
};

export type SettingsUpdateItem = {
  keyName: string;
  value: string;
};
