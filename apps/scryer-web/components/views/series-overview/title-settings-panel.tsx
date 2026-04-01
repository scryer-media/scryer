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
import { SubtitleLanguagePicker } from "@/components/common/subtitle-language-picker";
import { convenienceSettingsQuery } from "@/lib/graphql/queries";
import { setTitleRequiredAudioMutation } from "@/lib/graphql/mutations";
import { useTranslate } from "@/lib/context/translate-context";
import { getSubtitleLanguage } from "@/lib/constants/subtitle-languages";
import type { TitleDetail } from "@/components/containers/series-overview-container";
import type { TitleOptionUpdates } from "@/lib/types/title-options";

const INHERIT_VALUE = "__inherit__";
const DEFAULT_MARKER = "__default__";

export function TitleSettingsPanel({
  title,
  qualityProfiles,
  defaultRootFolder,
  rootFolders,
  onUpdateTitleOptions,
  onOpenFixMatch,
}: {
  title: TitleDetail;
  qualityProfiles: { id: string; name: string }[];
  defaultRootFolder: string;
  rootFolders: { path: string; isDefault: boolean }[];
  onUpdateTitleOptions: (options: TitleOptionUpdates) => Promise<void>;
  onOpenFixMatch?: () => void;
}) {
  const t = useTranslate();
  const client = useClient();

  const [requiredAudioLanguages, setRequiredAudioLanguages] = React.useState<string[]>([]);
  const [inheritedAudioLanguages, setInheritedAudioLanguages] = React.useState<string[]>([]);
  const [hasAudioOverride, setHasAudioOverride] = React.useState(false);
  const [audioLoaded, setAudioLoaded] = React.useState(false);

  React.useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const { data } = await client.query(convenienceSettingsQuery, {}).toPromise();
        if (cancelled || !data?.convenienceSettings) return;
        const settings = data.convenienceSettings.requiredAudio as {
          scope: string;
          languages: string[];
        }[];
        const titleScope = `title:${title.id}`;
        const titleMatch = settings.find((r) => r.scope === titleScope);
        const facetMatch = settings.find((r) => r.scope === title.facet);
        setInheritedAudioLanguages(facetMatch?.languages ?? []);
        if (titleMatch) {
          setRequiredAudioLanguages(titleMatch.languages);
          setHasAudioOverride(true);
        } else {
          setRequiredAudioLanguages(facetMatch?.languages ?? []);
          setHasAudioOverride(false);
        }
        setAudioLoaded(true);
      } catch {
        // silently ignore — non-critical
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [client, title.id, title.facet]);

  const handleRequiredAudioChange = async (languages: string[]) => {
    setRequiredAudioLanguages(languages);
    setHasAudioOverride(true);
    try {
      await client
        .mutation(setTitleRequiredAudioMutation, {
          input: { titleId: title.id, facet: title.facet, languages },
        })
        .toPromise();
    } catch {
      // silently ignore
    }
  };

  const handleResetAudioOverride = async () => {
    setRequiredAudioLanguages(inheritedAudioLanguages);
    setHasAudioOverride(false);
    try {
      await client
        .mutation(setTitleRequiredAudioMutation, {
          input: { titleId: title.id, facet: title.facet, languages: [] },
        })
        .toPromise();
    } catch {
      // silently ignore
    }
  };

  const formatLanguageList = (codes: string[]) =>
    codes.length === 0
      ? "None"
      : codes.map((c) => getSubtitleLanguage(c)?.name ?? c).join(", ");

  const currentProfileId = title.qualityProfileId?.trim() || INHERIT_VALUE;
  const currentRootFolder = title.rootFolderPath?.trim() || "";
  const currentSeasonFolder = title.useSeasonFolders === false ? "disabled" : "enabled";
  const currentFillerPolicy = title.fillerPolicy?.trim() || INHERIT_VALUE;
  const currentRecapPolicy = title.recapPolicy?.trim() || INHERIT_VALUE;
  const [saving, setSaving] = React.useState(false);

  const rootFolderSelectValue = currentRootFolder || DEFAULT_MARKER;

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

      {audioLoaded ? (
        <div className="mt-4 space-y-2">
          <label className="block text-xs font-medium text-muted-foreground">
            {t("title.requiredAudioLanguages")}
          </label>
          <SubtitleLanguagePicker
            value={requiredAudioLanguages}
            onChange={(codes) => void handleRequiredAudioChange(codes)}
          />
          {hasAudioOverride ? (
            <button
              type="button"
              className="text-xs text-primary hover:underline"
              onClick={() => void handleResetAudioOverride()}
            >
              {t("title.requiredAudioResetInherit")}
            </button>
          ) : (
            <p className="text-xs text-muted-foreground">
              {t("title.requiredAudioInherited", { facet: title.facet })}
              {inheritedAudioLanguages.length > 0
                ? `: ${formatLanguageList(inheritedAudioLanguages)}`
                : null}
            </p>
          )}
        </div>
      ) : null}

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
            variant="outline"
            size="sm"
            className="shrink-0"
            onClick={onOpenFixMatch}
          >
            <Search className="mr-2 h-4 w-4" />
            {t("title.fixMatchAction")}
          </Button>
        </div>
      ) : null}
    </div>
  );
}
