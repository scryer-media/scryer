import * as React from "react";
import { Check, Copy, Sparkles } from "lucide-react";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { IconButton } from "@/components/ui/icon-button";
import { Label } from "@/components/ui/label";
import { TextActionButton } from "@/components/ui/text-action-button";
import { Textarea } from "@/components/ui/textarea";
import { useTranslate } from "@/lib/context/translate-context";

const PROMPT_PREVIEW_MAX_LINES = 30;

type RegoAiPromptDialogProps = {
  id: string;
  ruleKind: string;
  inputContract: unknown;
  outputContract: string;
};

export function buildRegoAiPrompt({
  ruleKind,
  inputContract,
  outputContract,
  desiredOutcome,
}: Omit<RegoAiPromptDialogProps, "id"> & { desiredOutcome: string }) {
  return `Write a Scryer ${ruleKind} rule in Rego v1.

Desired outcome:
${desiredOutcome.trim() || "<Describe the behavior you want the rule to produce.>"}

Use only the fields defined in the input contract below. Do not invent fields, helper functions, packages, or data sources. Treat missing optional fields safely. Return only complete Rego source that can be pasted into the editor; do not include Markdown fences or an explanation.

Output contract:
${outputContract}

Input contract (the complete data available as \`input\`):
\`\`\`json
${JSON.stringify(inputContract, null, 2)}
\`\`\``;
}

export function RegoAiPromptDialog({
  id,
  ruleKind,
  inputContract,
  outputContract,
}: RegoAiPromptDialogProps) {
  const t = useTranslate();
  const [open, setOpen] = React.useState(false);
  const [desiredOutcome, setDesiredOutcome] = React.useState("");
  const [copied, setCopied] = React.useState(false);
  const copyResetTimerRef = React.useRef<ReturnType<typeof setTimeout> | null>(
    null,
  );
  const prompt = React.useMemo(
    () =>
      buildRegoAiPrompt({
        ruleKind,
        inputContract,
        outputContract,
        desiredOutcome,
      }),
    [desiredOutcome, inputContract, outputContract, ruleKind],
  );
  const promptPreview = React.useMemo(() => {
    const lines = prompt.split("\n");
    if (lines.length <= PROMPT_PREVIEW_MAX_LINES) {
      return prompt;
    }
    return `${lines.slice(0, PROMPT_PREVIEW_MAX_LINES).join("\n")}\n…`;
  }, [prompt]);

  React.useEffect(
    () => () => {
      if (copyResetTimerRef.current) {
        clearTimeout(copyResetTimerRef.current);
      }
    },
    [],
  );

  const copyPrompt = React.useCallback(async () => {
    if (!navigator.clipboard) {
      return;
    }

    await navigator.clipboard.writeText(prompt);
    setCopied(true);
    if (copyResetTimerRef.current) {
      clearTimeout(copyResetTimerRef.current);
    }
    copyResetTimerRef.current = setTimeout(() => setCopied(false), 1500);
  }, [prompt]);

  return (
    <Dialog open={open} onOpenChange={setOpen}>
      <TextActionButton
        id={id}
        tone="accentBright"
        size="default"
        onClick={() => setOpen(true)}
        leadingIcon={<Sparkles className="h-4 w-4" />}
        label={t("settings.ruleAiPrompt")}
      />
      <DialogContent className="max-h-[calc(100vh-2rem)] overflow-y-auto sm:max-w-[48rem]">
        <DialogHeader>
          <DialogTitle>{t("settings.ruleAiPromptTitle")}</DialogTitle>
          <DialogDescription>
            {t("settings.ruleAiPromptDescription")}
          </DialogDescription>
        </DialogHeader>
        <div className="space-y-2">
          <Label htmlFor={`${id}-outcome`}>
            {t("settings.ruleAiPromptOutcome")}
          </Label>
          <Textarea
            id={`${id}-outcome`}
            value={desiredOutcome}
            onChange={(event) => setDesiredOutcome(event.target.value)}
            placeholder={t("settings.ruleAiPromptOutcomePlaceholder")}
            rows={3}
          />
        </div>
        <div className="space-y-2">
          <Label id={`${id}-prompt-label`}>{t("settings.ruleAiPrompt")}</Label>
          <div className="relative">
            <pre
              id={`${id}-prompt`}
              aria-labelledby={`${id}-prompt-label`}
              className="border-input bg-input w-full overflow-hidden rounded-md border py-2 pr-14 pl-3 font-mono text-xs leading-5 break-words whitespace-pre-wrap"
            >
              {promptPreview}
            </pre>
            <IconButton
              id={`${id}-copy`}
              label={
                copied
                  ? t("settings.ruleAiPromptCopied")
                  : t("settings.ruleAiPromptCopy")
              }
              className="absolute top-2 right-2"
              onClick={() => void copyPrompt()}
            >
              {copied ? (
                <Check className="h-4 w-4" />
              ) : (
                <Copy className="h-4 w-4" />
              )}
            </IconButton>
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}
