import * as React from "react";
import { FolderOpen } from "lucide-react";

import type { ScriptInterpreterDraft } from "@/components/containers/settings/settings-scripts-container";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { FolderBrowserDialog } from "@/components/setup/folder-browser-dialog";
import { useTranslate } from "@/lib/context/translate-context";

const SCRIPTS_PANEL_CLASS =
  "overflow-hidden rounded-[14px] border border-[var(--scry-border)] bg-[var(--scry-surf)] shadow-[0_10px_24px_rgba(0,0,0,0.16)]";
const SCRIPTS_PANEL_HEADER_CLASS =
  "border-b border-[var(--scry-border3)] bg-[linear-gradient(180deg,rgba(255,255,255,0.035),rgba(255,255,255,0))] px-4 py-3";
const SCRIPTS_PANEL_TITLE_CLASS = "text-[15px] font-semibold text-[var(--scry-ink2)]";
const SCRIPTS_PANEL_BODY_CLASS = "p-4 sm:p-5";
const SCRIPTS_MUTED_TEXT_CLASS = "text-[var(--scry-muted3)]";

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
  // One dialog serves every field; this names the field it fills.
  const [browsingKey, setBrowsingKey] = React.useState<keyof ScriptInterpreterDraft | null>(null);
  return (
    <div id="settings-scripts-section" className="space-y-4 text-sm">
      <section id="settings-script-interpreters" className={SCRIPTS_PANEL_CLASS}>
        <div className={SCRIPTS_PANEL_HEADER_CLASS}>
          <h2 className={SCRIPTS_PANEL_TITLE_CLASS}>
            {t("settings.scriptInterpreters.title")}
          </h2>
          <p className={`mt-1 text-[12.5px] ${SCRIPTS_MUTED_TEXT_CLASS}`}>
            {t("settings.scriptInterpreters.description")}
          </p>
        </div>
        <div className={SCRIPTS_PANEL_BODY_CLASS}>
          <form
            className="space-y-4"
            onSubmit={(event) => {
              event.preventDefault();
              void onSave();
            }}
          >
            {/* Paths are short, so the fields stop well before a wide panel does. */}
            <div className="max-w-xl space-y-4">
              {INTERPRETER_FIELDS.map(({ key, placeholder }) => {
                const inputId = `settings-script-interpreter-${key}`;
                return (
                  <div key={key} className="space-y-1.5">
                    <Label className="text-[var(--scry-ink2)]" htmlFor={inputId}>
                      {t(`settings.scriptInterpreters.${key}`)}
                    </Label>
                    <div className="flex gap-2">
                      <Input
                        id={inputId}
                        value={draft[key]}
                        onChange={(event) =>
                          setDraft((prev) => ({ ...prev, [key]: event.target.value }))
                        }
                        className="font-[var(--font-code)]"
                        placeholder={placeholder}
                        autoComplete="off"
                        spellCheck={false}
                      />
                      <Button
                        id={`${inputId}-browse`}
                        type="button"
                        variant="outline"
                        aria-haspopup="dialog"
                        onClick={() => setBrowsingKey(key)}
                      >
                        <FolderOpen className="mr-1 h-4 w-4" />
                        {t("setup.browse")}
                      </Button>
                    </div>
                    {key === "powershell" ? (
                      <p className={`text-xs ${SCRIPTS_MUTED_TEXT_CLASS}`}>
                        {t("settings.scriptInterpreters.powershellHelp")}
                      </p>
                    ) : null}
                  </div>
                );
              })}
            </div>
            <p className={`text-xs ${SCRIPTS_MUTED_TEXT_CLASS}`}>
              {t("settings.scriptInterpreters.containerHelp")}
            </p>
            <div className="flex flex-wrap gap-2 pt-2">
              <Button
                id="settings-script-interpreters-save"
                type="submit"
                disabled={isSaving || !isDirty}
              >
                {isSaving ? t("label.saving") : t("label.save")}
              </Button>
            </div>
          </form>
        </div>
      </section>
      {browsingKey ? (
        <FolderBrowserDialog
          open
          onOpenChange={(open) => {
            if (!open) setBrowsingKey(null);
          }}
          onSelect={(path) => setDraft((prev) => ({ ...prev, [browsingKey]: path }))}
          selectionTypes={["file"]}
          initialPath={
            draft[browsingKey].startsWith("/")
              ? draft[browsingKey].replace(/\/[^/]+$/, "") || "/"
              : "/"
          }
          title={t("settings.scriptInterpreters.selectFile", {
            language: t(`settings.scriptInterpreters.${browsingKey}`),
          })}
        />
      ) : null}
    </div>
  );
}
