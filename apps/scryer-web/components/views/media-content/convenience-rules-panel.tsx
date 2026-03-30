import * as React from "react";
import { useClient } from "urql";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Label } from "@/components/ui/label";
import { SubtitleLanguagePicker } from "@/components/common/subtitle-language-picker";
import { useTranslate } from "@/lib/context/translate-context";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { convenienceSettingsQuery } from "@/lib/graphql/queries";
import { setConvenienceRequiredAudioMutation } from "@/lib/graphql/mutations";
import type { ViewId } from "@/components/root/types";

type ConvenienceRulesPanelProps = {
  view: ViewId;
};

type RequiredAudioSetting = {
  scope: string;
  languages: string[];
  ruleSetId: string | null;
};

type ConvenienceSettings = {
  requiredAudio: RequiredAudioSetting[];
};

function viewToScope(view: ViewId): string {
  if (view === "movies") return "movie";
  if (view === "series") return "series";
  if (view === "anime") return "anime";
  return "movie";
}

export function ConvenienceRulesPanel({ view }: ConvenienceRulesPanelProps) {
  const t = useTranslate();
  const setGlobalStatus = useGlobalStatus();
  const client = useClient();
  const scope = viewToScope(view);

  const [settings, setSettings] = React.useState<ConvenienceSettings | null>(null);
  const [loading, setLoading] = React.useState(true);

  const loadSettings = React.useCallback(async () => {
    try {
      const { data, error } = await client.query(convenienceSettingsQuery, {}).toPromise();
      if (error) throw error;
      setSettings(data.convenienceSettings ?? null);
    } catch (err) {
      setGlobalStatus(err instanceof Error ? err.message : t("status.failedToLoad"));
    } finally {
      setLoading(false);
    }
  }, [client, setGlobalStatus, t]);

  React.useEffect(() => {
    void loadSettings();
  }, [loadSettings]);

  const currentRequiredAudio = React.useMemo(() => {
    if (!settings) return [];
    const match = settings.requiredAudio.find((r) => r.scope === scope);
    return match?.languages ?? [];
  }, [settings, scope]);

  const handleRequiredAudioChange = React.useCallback(
    async (languages: string[]) => {
      try {
        const { error } = await client
          .mutation(setConvenienceRequiredAudioMutation, {
            input: { scope, languages },
          })
          .toPromise();
        if (error) throw error;
        // Optimistic update
        setSettings((prev) => {
          if (!prev) return prev;
          const updated = prev.requiredAudio.filter((r) => r.scope !== scope);
          if (languages.length > 0) {
            updated.push({ scope, languages, ruleSetId: null });
          }
          return { ...prev, requiredAudio: updated };
        });
      } catch (err) {
        setGlobalStatus(err instanceof Error ? err.message : t("status.failedToUpdate"));
      }
    },
    [client, scope, setGlobalStatus, t],
  );

  if (loading) {
    return null;
  }

  return (
    <Card>
      <CardHeader>
        <CardTitle>{t("convenience.title")}</CardTitle>
      </CardHeader>
      <CardContent className="space-y-4">
        <div className="space-y-2">
          <Label className="text-sm text-card-foreground">
            {t("convenience.requiredAudioLabel")}
          </Label>
          <SubtitleLanguagePicker
            value={currentRequiredAudio}
            onChange={(codes) => void handleRequiredAudioChange(codes)}
          />
          <p className="text-xs text-muted-foreground">
            {t("convenience.requiredAudioHelp")}
          </p>
        </div>
      </CardContent>
    </Card>
  );
}
