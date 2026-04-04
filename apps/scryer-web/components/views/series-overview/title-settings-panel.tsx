import * as React from "react";
import { useClient } from "urql";
import { Search } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { MediaRenamePlanPanel } from "@/components/common/media-rename-plan-panel";
import { SubtitleLanguagePicker } from "@/components/common/subtitle-language-picker";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { mediaRenamePreviewQuery } from "@/lib/graphql/queries";
import { applyMediaRenameMutation, setTitleRequiredAudioMutation } from "@/lib/graphql/mutations";
import { useTranslate } from "@/lib/context/translate-context";
import type { TitleDetail } from "@/components/containers/series-overview-container";
import type { TitleOptionUpdates } from "@/lib/types/title-options";

const INHERIT_VALUE = "__inherit__";
const DEFAULT_MARKER = "__default__";

type MediaRenamePlanItem = {
  collectionId: string | null;
  currentPath: string;
  proposedPath: string | null;
};

type MediaRenamePlan = {
  fingerprint: string;
  total: number;
  renamable: number;
  noop: number;
  conflicts: number;
  errors: number;
  items: MediaRenamePlanItem[];
};

export function TitleSettingsPanel({
  title,
  qualityProfiles,
  defaultRootFolder,
  rootFolders,
  onUpdateTitleOptions,
  onOpenFixMatch,
  onTitleChanged,
}: {
  title: TitleDetail;
  qualityProfiles: { id: string; name: string }[];
  defaultRootFolder: string;
  rootFolders: { path: string; isDefault: boolean }[];
  onUpdateTitleOptions: (options: TitleOptionUpdates) => Promise<void>;
  onOpenFixMatch?: () => void;
  onTitleChanged?: () => Promise<void> | void;
}) {
  const t = useTranslate();
  const client = useClient();
  const setGlobalStatus = useGlobalStatus();
  const requiredAudioLanguages =
    title.effectiveRequiredAudioLanguages ?? [];
  const hasAudioOverride = title.inheritsRequiredAudioLanguages === false;
  const [renamePlan, setRenamePlan] = React.useState<MediaRenamePlan | null>(null);
  const [renamePreviewing, setRenamePreviewing] = React.useState(false);
  const [renameApplying, setRenameApplying] = React.useState(false);
  const [audioSaving, setAudioSaving] = React.useState(false);

  const handleRequiredAudioChange = async (languages: string[]) => {
    setAudioSaving(true);
    try {
      const { error } = await client
        .mutation(setTitleRequiredAudioMutation, {
          input: { titleId: title.id, facet: title.facet, languages },
        })
        .toPromise();
      if (error) {
        throw error;
      }
      await onTitleChanged?.();
    } catch {
      setGlobalStatus(t("status.failedToUpdate"));
    } finally {
      setAudioSaving(false);
    }
  };

  const handleResetAudioOverride = async () => {
    setAudioSaving(true);
    try {
      const { error } = await client
        .mutation(setTitleRequiredAudioMutation, {
          input: { titleId: title.id, facet: title.facet, languages: null },
        })
        .toPromise();
      if (error) {
        throw error;
      }
      await onTitleChanged?.();
    } catch {
      setGlobalStatus(t("status.failedToUpdate"));
    } finally {
      setAudioSaving(false);
    }
  };

  const currentProfileId = title.qualityProfileId?.trim() || INHERIT_VALUE;
  const currentRootFolder = title.rootFolderPath?.trim() || "";
  const currentSeasonFolder = title.useSeasonFolders === false ? "disabled" : "enabled";
  const currentFillerPolicy = title.fillerPolicy?.trim() || INHERIT_VALUE;
  const currentRecapPolicy = title.recapPolicy?.trim() || INHERIT_VALUE;
  const [saving, setSaving] = React.useState(false);

  const rootFolderSelectValue = currentRootFolder || DEFAULT_MARKER;

  React.useEffect(() => {
    setRenamePlan(null);
  }, [title.id, title.facet]);

  const handleProfileChange = async (value: string) => {
    setSaving(true);
    try {
      await onUpdateTitleOptions({
        qualityProfileId: value === INHERIT_VALUE ? "" : value,
      });
    } finally {
      setSaving(false);
    }
  };

  const handleRootFolderChange = async (value: string) => {
    setSaving(true);
    try {
      await onUpdateTitleOptions({
        rootFolderPath: value === DEFAULT_MARKER ? "" : value,
      });
    } finally {
      setSaving(false);
    }
  };

  const handleSeasonFolderChange = async (value: string) => {
    setSaving(true);
    try {
      await onUpdateTitleOptions({
        useSeasonFolders: value === "enabled",
      });
    } finally {
      setSaving(false);
    }
  };

  const handleFillerPolicyChange = async (value: string) => {
    setSaving(true);
    try {
      await onUpdateTitleOptions({
        fillerPolicy: value === INHERIT_VALUE ? "" : value,
      });
    } finally {
      setSaving(false);
    }
  };

  const handleRecapPolicyChange = async (value: string) => {
    setSaving(true);
    try {
      await onUpdateTitleOptions({
        recapPolicy: value === INHERIT_VALUE ? "" : value,
      });
    } finally {
      setSaving(false);
    }
  };

  const folderLabel = (path: string) =>
    path.split("/").filter(Boolean).pop() ?? path;

  const handlePreviewRename = async () => {
    setRenamePreviewing(true);
    try {
      const { data, error } = await client.query(mediaRenamePreviewQuery, {
        input: {
          facet: title.facet,
          titleId: title.id,
          dryRun: true,
        },
      }).toPromise();
      if (error) throw error;
      const plan = data.mediaRenamePreview as MediaRenamePlan;
      setRenamePlan(plan);
      setGlobalStatus(
        t("status.renamePreviewGenerated", {
          total: plan.total,
          renamable: plan.renamable,
        }),
      );
    } catch (error: unknown) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.apiError"));
      setRenamePlan(null);
    } finally {
      setRenamePreviewing(false);
    }
  };

  const handleApplyRename = async () => {
    if (!renamePlan) return;
    setRenameApplying(true);
    try {
      const { data, error } = await client.mutation(applyMediaRenameMutation, {
        input: {
          facet: title.facet,
          titleId: title.id,
          fingerprint: renamePlan.fingerprint,
        },
      }).toPromise();
      if (error) throw error;
      const result = data.applyMediaRename as {
        applied: number;
        skipped: number;
        failed: number;
      };
      setGlobalStatus(
        t("status.renameApplied", {
          applied: result.applied,
          skipped: result.skipped,
          failed: result.failed,
        }),
      );
      setRenamePlan(null);
      await onTitleChanged?.();
    } catch (error: unknown) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.apiError"));
    } finally {
      setRenameApplying(false);
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

        <div className="min-w-0">
          <label className="mb-1 block text-xs font-medium text-muted-foreground">
            {t("title.rootFolder")}
          </label>
          <Select
            value={rootFolderSelectValue}
            onValueChange={(v) => void handleRootFolderChange(v)}
            disabled={saving}
          >
            <SelectTrigger className="h-9 w-full font-mono text-sm">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value={DEFAULT_MARKER}>
                {t("title.defaultRootFolder", { path: folderLabel(defaultRootFolder) })}
              </SelectItem>
              {rootFolders
                .filter((rf) => !rf.isDefault)
                .map((rf) => (
                  <SelectItem key={rf.path} value={rf.path}>
                    {folderLabel(rf.path)}
                  </SelectItem>
                ))}
            </SelectContent>
          </Select>
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

        <div className="min-w-0 xl:max-w-72">
          <label className="mb-1 block text-xs font-medium text-muted-foreground">
            {t("title.requiredAudioLanguages")}
          </label>
          <SubtitleLanguagePicker
            value={requiredAudioLanguages}
            onChange={(codes) => void handleRequiredAudioChange(codes)}
            compact
            disabled={audioSaving}
          />
          {hasAudioOverride ? (
            <button
              type="button"
              className="mt-1 text-xs text-primary hover:underline"
              onClick={() => void handleResetAudioOverride()}
              disabled={audioSaving}
            >
              {t("title.requiredAudioResetInherit")}
            </button>
          ) : null}
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

      {onOpenFixMatch ? (
        <div className="mt-5 flex items-center justify-between gap-3 rounded-lg border border-border/70 bg-muted/20 px-3 py-3">
          <div className="min-w-0">
            <p className="text-sm font-medium text-foreground">{t("title.fixMatchHeading")}</p>
            <p className="text-xs text-muted-foreground">
              {t("title.fixMatchDescriptionSeries")}
            </p>
          </div>
          <Button
            type="button"
            variant="primary"
            size="sm"
            className="shrink-0"
            onClick={onOpenFixMatch}
          >
            <Search className="mr-2 h-4 w-4" />
            {t("title.fixMatchAction")}
          </Button>
        </div>
      ) : null}

      <div className={`${onOpenFixMatch ? "mt-3" : "mt-5"} rounded-lg border border-border/70 bg-muted/20 px-3 py-3`}>
        <div className="flex justify-end">
          <Button
            type="button"
            variant="primary"
            size="sm"
            className="shrink-0"
            onClick={() => void handlePreviewRename()}
            disabled={saving || renamePreviewing || renameApplying}
          >
            {renamePreviewing ? t("rename.previewing") : t("rename.previewButton")}
          </Button>
        </div>

        {renamePlan ? (
          <MediaRenamePlanPanel
            plan={renamePlan}
            applying={renameApplying}
            applyDisabled={renameApplying || renamePreviewing || renamePlan.renamable === 0}
            onApply={() => void handleApplyRename()}
          />
        ) : null}
      </div>
    </div>
  );
}
