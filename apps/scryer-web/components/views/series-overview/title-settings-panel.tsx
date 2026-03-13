import * as React from "react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  QUALITY_PROFILE_PREFIX,
  ROOT_FOLDER_PREFIX,
  SEASON_FOLDER_PREFIX,
  FILLER_POLICY_PREFIX,
  RECAP_POLICY_PREFIX,
  getTagValue,
  setTagValue,
  removeTagByPrefix,
} from "@/lib/utils/title-tags";
import { useTranslate } from "@/lib/context/translate-context";
import type { TitleDetail } from "@/components/containers/series-overview-container";

const INHERIT_VALUE = "__inherit__";

export function TitleSettingsPanel({
  title,
  qualityProfiles,
  defaultRootFolder,
  onUpdateTitleTags,
}: {
  title: TitleDetail;
  qualityProfiles: { id: string; name: string }[];
  defaultRootFolder: string;
  onUpdateTitleTags: (newTags: string[]) => Promise<void>;
}) {
  const t = useTranslate();
  const currentProfileId = getTagValue(title.tags, QUALITY_PROFILE_PREFIX) ?? INHERIT_VALUE;
  const currentRootFolder = getTagValue(title.tags, ROOT_FOLDER_PREFIX) ?? "";
  const currentSeasonFolder = getTagValue(title.tags, SEASON_FOLDER_PREFIX) ?? "enabled";
  const currentFillerPolicy = getTagValue(title.tags, FILLER_POLICY_PREFIX) ?? INHERIT_VALUE;
  const currentRecapPolicy = getTagValue(title.tags, RECAP_POLICY_PREFIX) ?? INHERIT_VALUE;
  const [rootFolderDraft, setRootFolderDraft] = React.useState(currentRootFolder || defaultRootFolder);
  const [saving, setSaving] = React.useState(false);

  React.useEffect(() => {
    setRootFolderDraft(currentRootFolder || defaultRootFolder);
  }, [currentRootFolder, defaultRootFolder]);

  const handleProfileChange = async (value: string) => {
    setSaving(true);
    try {
      const newTags =
        value === INHERIT_VALUE
          ? removeTagByPrefix(title.tags, QUALITY_PROFILE_PREFIX)
          : setTagValue(title.tags, QUALITY_PROFILE_PREFIX, value);
      await onUpdateTitleTags(newTags);
    } finally {
      setSaving(false);
    }
  };

  const handleRootFolderSave = async () => {
    const trimmed = rootFolderDraft.trim();
    if (!trimmed || trimmed === defaultRootFolder) {
      setSaving(true);
      try {
        await onUpdateTitleTags(removeTagByPrefix(title.tags, ROOT_FOLDER_PREFIX));
      } finally {
        setSaving(false);
      }
      return;
    }
    setSaving(true);
    try {
      await onUpdateTitleTags(setTagValue(title.tags, ROOT_FOLDER_PREFIX, trimmed));
    } finally {
      setSaving(false);
    }
  };

  const handleSeasonFolderChange = async (value: string) => {
    setSaving(true);
    try {
      await onUpdateTitleTags(setTagValue(title.tags, SEASON_FOLDER_PREFIX, value));
    } finally {
      setSaving(false);
    }
  };

  const handleFillerPolicyChange = async (value: string) => {
    setSaving(true);
    try {
      const newTags =
        value === INHERIT_VALUE
          ? removeTagByPrefix(title.tags, FILLER_POLICY_PREFIX)
          : setTagValue(title.tags, FILLER_POLICY_PREFIX, value);
      await onUpdateTitleTags(newTags);
    } finally {
      setSaving(false);
    }
  };

  const handleRecapPolicyChange = async (value: string) => {
    setSaving(true);
    try {
      const newTags =
        value === INHERIT_VALUE
          ? removeTagByPrefix(title.tags, RECAP_POLICY_PREFIX)
          : setTagValue(title.tags, RECAP_POLICY_PREFIX, value);
      await onUpdateTitleTags(newTags);
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="p-4">
      <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-4">
        <div className="min-w-0">
          <label className="mb-1 block text-xs font-medium text-muted-foreground">
            {t("title.qualityProfile")}
          </label>
          <Select
            value={currentProfileId}
            onValueChange={(v) => void handleProfileChange(v)}
            disabled={saving || qualityProfiles.length === 0}
          >
            <SelectTrigger className="h-9 w-full">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value={INHERIT_VALUE}>
                {t("title.inheritDefault")}
              </SelectItem>
              {qualityProfiles.map((p) => (
                <SelectItem key={p.id} value={p.id}>
                  {p.name}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>

        <div className="min-w-0 xl:col-span-2">
          <label className="mb-1 block text-xs font-medium text-muted-foreground">
            {t("title.rootFolder")}
          </label>
          <div className="flex flex-col gap-2 sm:flex-row">
            <Input
              className="h-9 font-mono text-sm"
              value={rootFolderDraft}
              onChange={(e) => setRootFolderDraft(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") void handleRootFolderSave();
              }}
              disabled={saving}
            />
            {rootFolderDraft.trim() !== (currentRootFolder || defaultRootFolder) && (
              <Button
                size="sm"
                className="h-9 sm:self-auto"
                onClick={() => void handleRootFolderSave()}
                disabled={saving}
              >
                {t("settings.saveButton")}
              </Button>
            )}
          </div>
        </div>

        <div className="min-w-0">
          <label className="mb-1 block text-xs font-medium text-muted-foreground">
            {t("search.addConfigSeasonFolder")}
          </label>
          <Select
            value={currentSeasonFolder}
            onValueChange={(v) => void handleSeasonFolderChange(v)}
            disabled={saving}
          >
            <SelectTrigger className="h-9 w-full">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="enabled">{t("search.seasonFolder.enabled")}</SelectItem>
              <SelectItem value="disabled">{t("search.seasonFolder.disabled")}</SelectItem>
            </SelectContent>
          </Select>
        </div>

        {title.facet === "anime" ? (
          <>
            <div className="min-w-0">
              <label className="mb-1 block text-xs font-medium text-muted-foreground">
                {t("settings.fillerPolicyLabel")}
              </label>
              <Select
                value={currentFillerPolicy}
                onValueChange={(v) => void handleFillerPolicyChange(v)}
                disabled={saving}
              >
                <SelectTrigger className="h-9 w-full">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value={INHERIT_VALUE}>{t("title.inheritDefault")}</SelectItem>
                  <SelectItem value="download_all">{t("settings.fillerPolicyDownloadAll")}</SelectItem>
                  <SelectItem value="skip_filler">{t("settings.fillerPolicySkipFiller")}</SelectItem>
                </SelectContent>
              </Select>
            </div>

            <div className="min-w-0">
              <label className="mb-1 block text-xs font-medium text-muted-foreground">
                {t("settings.recapPolicyLabel")}
              </label>
              <Select
                value={currentRecapPolicy}
                onValueChange={(v) => void handleRecapPolicyChange(v)}
                disabled={saving}
              >
                <SelectTrigger className="h-9 w-full">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value={INHERIT_VALUE}>{t("title.inheritDefault")}</SelectItem>
                  <SelectItem value="download_all">{t("settings.recapPolicyDownloadAll")}</SelectItem>
                  <SelectItem value="skip_recap">{t("settings.recapPolicySkipRecap")}</SelectItem>
                </SelectContent>
              </Select>
            </div>
          </>
        ) : null}
      </div>
    </div>
  );
}
