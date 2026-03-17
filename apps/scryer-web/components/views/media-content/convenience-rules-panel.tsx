import * as React from "react";
import { useClient } from "urql";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Label } from "@/components/ui/label";
import { SubtitleLanguagePicker } from "@/components/common/subtitle-language-picker";
import { useTranslate } from "@/lib/context/translate-context";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { convenienceSettingsQuery } from "@/lib/graphql/queries";
import {
  setConvenienceRequiredAudioMutation,
  setConveniencePreferDualAudioMutation,
} from "@/lib/graphql/mutations";
import type { ViewId } from "@/components/root/types";

type ConvenienceRulesPanelProps = {
  view: ViewId;
};

type RequiredAudioSetting = {
  scope: string;
  languages: string[];
  ruleSetId: string | null;
};

type PreferDualAudioSetting = {
  scope: string;
  enabled: boolean;
  ruleSetId: string | null;
};

type ConvenienceSettings = {
  requiredAudio: RequiredAudioSetting[];
  preferDualAudio: PreferDualAudioSetting[];
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
  const [saving, setSaving] = React.useState(false);

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

  const currentPreferDualAudio = React.useMemo(() => {
    if (!settings) return false;
    const match = settings.preferDualAudio.find((r) => r.scope === scope);
    return match?.enabled ?? false;
  }, [settings, scope]);

  const handleRequiredAudioChange = React.useCallback(
    async (languages: string[]) => {
      setSaving(true);
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
      } finally {
        setSaving(false);
      }
    },
    [client, scope, setGlobalStatus, t],
  );

  const handlePreferDualAudioChange = React.useCallback(
    async (enabled: boolean) => {
      setSaving(true);
      try {
        const { error } = await client
          .mutation(setConveniencePreferDualAudioMutation, {
            input: { scope, enabled },
          })
          .toPromise();
        if (error) throw error;
        // Optimistic update
        setSettings((prev) => {
          if (!prev) return prev;
          const updated = prev.preferDualAudio.filter((r) => r.scope !== scope);
          updated.push({ scope, enabled, ruleSetId: null });
          return { ...prev, preferDualAudio: updated };
        });
      } catch (err) {
        setGlobalStatus(err instanceof Error ? err.message : t("status.failedToUpdate"));
      } finally {
        setSaving(false);
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

        <div className="space-y-2">
          <Label className="text-sm text-card-foreground">
            {t("convenience.preferDualAudioLabel")}
          </Label>
          <div className="flex items-center gap-3">
            <button
              type="button"
              role="switch"
              aria-checked={currentPreferDualAudio}
              className={`relative inline-flex h-6 w-11 shrink-0 cursor-pointer rounded-full border-2 border-transparent transition-colors ${currentPreferDualAudio ? "bg-primary" : "bg-muted"}`}
              onClick={() => void handlePreferDualAudioChange(!currentPreferDualAudio)}
              disabled={saving}
            >
              <span
                className={`pointer-events-none inline-block h-5 w-5 rounded-full bg-background shadow-lg transition-transform ${currentPreferDualAudio ? "translate-x-5" : "translate-x-0"}`}
              />
            </button>
            <span className="text-xs text-muted-foreground">
              {t("convenience.preferDualAudioHelp")}
            </span>
          </div>
        </div>
      </CardContent>
    </Card>
  );
}
