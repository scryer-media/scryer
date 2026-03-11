import { Button } from "@/components/ui/button";
import { Input, integerInputProps, sanitizeDigits } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Loader2 } from "lucide-react";
import { useTranslate } from "@/lib/context/translate-context";

type PostProcessingSettings = {
  movieScript: string;
  seriesScript: string;
  animeScript: string;
  timeoutSecs: number;
};

type Props = {
  settings: PostProcessingSettings;
  setSettings: (s: PostProcessingSettings) => void;
  saving: boolean;
  loading: boolean;
  onSave: () => void;
};

const ENV_VARS = [
  "SCRYER_EVENT",
  "SCRYER_FACET",
  "SCRYER_FILE_PATH",
  "SCRYER_TITLE_NAME",
  "SCRYER_TITLE_ID",
  "SCRYER_YEAR",
  "SCRYER_IMDB_ID",
  "SCRYER_TVDB_ID",
  "SCRYER_SEASON",
  "SCRYER_EPISODE",
  "SCRYER_QUALITY",
];

export function SettingsPostProcessingSection({
  settings,
  setSettings,
  saving,
  loading,
  onSave,
}: Props) {
  const t = useTranslate();
  const update = (patch: Partial<PostProcessingSettings>) =>
    setSettings({ ...settings, ...patch });

  if (loading) {
    return (
      <div className="flex items-center gap-2 text-sm text-muted-foreground">
        <Loader2 className="h-4 w-4 animate-spin" />
        {t("status.loading")}
      </div>
    );
  }

  return (
    <div className="space-y-6 text-sm">
      <p className="text-muted-foreground">{t("settings.pp.intro")}</p>

      <div className="space-y-4">
        <div className="space-y-1">
          <Label>{t("settings.pp.movieScript")}</Label>
          <Input
            className="font-mono"
            value={settings.movieScript}
            onChange={(e) => update({ movieScript: e.target.value })}
            placeholder='e.g. ffmpeg -i "$SCRYER_FILE_PATH" ...'
          />
        </div>
        <div className="space-y-1">
          <Label>{t("settings.pp.seriesScript")}</Label>
          <Input
            className="font-mono"
            value={settings.seriesScript}
            onChange={(e) => update({ seriesScript: e.target.value })}
            placeholder='e.g. ffmpeg -i "$SCRYER_FILE_PATH" ...'
          />
        </div>
        <div className="space-y-1">
          <Label>{t("settings.pp.animeScript")}</Label>
          <Input
            className="font-mono"
            value={settings.animeScript}
            onChange={(e) => update({ animeScript: e.target.value })}
            placeholder='e.g. ffmpeg -i "$SCRYER_FILE_PATH" ...'
          />
        </div>
        <div className="space-y-1">
          <Label>{t("settings.pp.timeoutSecs")}</Label>
          <Input
            {...integerInputProps}
            value={settings.timeoutSecs}
            onChange={(e) =>
              update({
                timeoutSecs: Number(sanitizeDigits(e.target.value)) || 0,
              })
            }
          />
        </div>
      </div>

      <div className="space-y-2">
        <p className="font-medium">{t("settings.pp.envVarsHeading")}</p>
        <p className="text-muted-foreground">{t("settings.pp.envVarsDescription")}</p>
        <div className="rounded-md border bg-muted/50 p-3 font-mono text-xs leading-relaxed">
          {ENV_VARS.map((v) => (
            <div key={v}>{v}</div>
          ))}
        </div>
      </div>

      <Button onClick={onSave} disabled={saving}>
        {saving ? (
          <>
            <Loader2 className="mr-2 h-4 w-4 animate-spin" />
            {t("settings.saving")}
          </>
        ) : (
          t("settings.save")
        )}
      </Button>
    </div>
  );
}
