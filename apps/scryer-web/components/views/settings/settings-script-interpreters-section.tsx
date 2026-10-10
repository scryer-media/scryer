import type * as React from "react";
import { Cpu } from "lucide-react";

import type { ScriptInterpreterDraft } from "@/components/containers/settings/settings-scripts-container";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { useTranslate } from "@/lib/context/translate-context";

const INTERPRETER_FIELDS: ReadonlyArray<{
  key: keyof ScriptInterpreterDraft;
  placeholder: string;
}> = [
  { key: "python", placeholder: "/usr/bin/python3" },
  { key: "powershell", placeholder: "/usr/bin/pwsh" },
  { key: "batch", placeholder: "C:\\Windows\\System32\\cmd.exe" },
  { key: "go", placeholder: "/usr/local/go/bin/go" },
];

export function SettingsScriptInterpretersSection({
  draft,
  setDraft,
  isDirty,
  isSaving,
  onSave,
}: {
  draft: ScriptInterpreterDraft;
  setDraft: React.Dispatch<React.SetStateAction<ScriptInterpreterDraft>>;
  isDirty: boolean;
  isSaving: boolean;
  onSave: () => Promise<void> | void;
}) {
  const t = useTranslate();
  return (
    <div id="settings-scripts-section" className="max-w-[880px] text-sm">
      <Card id="settings-script-interpreters">
        <CardHeader>
          <CardTitle className="flex items-center gap-2 text-base">
            <Cpu className="h-4 w-4" />
            {t("settings.scriptInterpreters.title")}
          </CardTitle>
          <p className="text-xs text-muted-foreground">
            {t("settings.scriptInterpreters.description")}
          </p>
        </CardHeader>
        <CardContent className="space-y-3 text-sm">
          <form
            className="space-y-3"
            onSubmit={(event) => {
              event.preventDefault();
              void onSave();
            }}
          >
            {INTERPRETER_FIELDS.map(({ key, placeholder }) => (
              <label key={key} className="block">
                <Label className="mb-2 block">{t(`settings.scriptInterpreters.${key}`)}</Label>
                <Input
                  id={`settings-script-interpreter-${key}`}
                  value={draft[key]}
                  onChange={(event) =>
                    setDraft((prev) => ({ ...prev, [key]: event.target.value }))
                  }
                  className="font-[var(--font-code)]"
                  placeholder={placeholder}
                  autoComplete="off"
                  spellCheck={false}
                />
              </label>
            ))}
            <p className="text-xs text-muted-foreground">
              {t("settings.scriptInterpreters.containerHelp")}
            </p>
            <p className="text-xs text-muted-foreground">
              {t("settings.scriptInterpreters.powershellHelp")}
            </p>
            <Button
              id="settings-script-interpreters-save"
              type="submit"
              size="sm"
              disabled={isSaving || !isDirty}
            >
              {isSaving ? t("label.saving") : t("label.save")}
            </Button>
          </form>
        </CardContent>
      </Card>
    </div>
  );
}
