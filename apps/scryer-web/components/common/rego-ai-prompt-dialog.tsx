import * as React from "react";
import { Check, Copy, Sparkles } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import { useTranslate } from "@/lib/context/translate-context";

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
      <Button
        id={id}
        type="button"
        variant="secondary"
        onClick={() => setOpen(true)}
      >
        <Sparkles className="mr-2 h-4 w-4" />
        {t("settings.ruleAiPrompt")}
      </Button>
      <DialogContent className="max-h-[calc(100vh-2rem)] max-w-[min(96vw,64rem)] overflow-y-auto">
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
          <Label htmlFor={`${id}-prompt`}>{t("settings.ruleAiPrompt")}</Label>
          <Textarea
            id={`${id}-prompt`}
            readOnly
            value={prompt}
            className="min-h-80 font-mono text-xs leading-5"
          />
        </div>
        <DialogFooter>
          <Button type="button" onClick={() => void copyPrompt()}>
            {copied ? (
              <Check className="mr-2 h-4 w-4" />
            ) : (
              <Copy className="mr-2 h-4 w-4" />
            )}
            {copied
              ? t("settings.ruleAiPromptCopied")
              : t("settings.ruleAiPromptCopy")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
