import * as React from "react";
import { Check, Copy, Sparkles } from "lucide-react";
import { toast } from "sonner";
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
import { copyText } from "@/lib/utils/clipboard";

const PROMPT_PREVIEW_MAX_LINES = 30;

type RegoInputContract = {
  sections: readonly {
    path: string;
    fields: readonly { field: string; type: string }[];
  }[];
};

type RegoAiPromptDialogProps = {
  id: string;
  ruleKind: string;
  inputContract: RegoInputContract;
  outputContract: string;
};

/// One line per input object, `path: field type, ...`. The contract's i18n
/// keys carry nothing a model can use, so only names and types are sent.
function compactInputContract({ sections }: RegoInputContract) {
  return sections
    .map(
      ({ path, fields }) =>
        `${path}: ${fields.map(({ field, type }) => `${field} ${type}`).join(", ")}`,
    )
    .join("\n");
}

export function buildRegoAiPrompt({
  ruleKind,
  inputContract,
  outputContract,
  desiredOutcome,
}: Omit<RegoAiPromptDialogProps, "id"> & { desiredOutcome: string }) {
  return `Write a Scryer ${ruleKind} rule in Rego v1. Reply with pasteable Rego only, no fences or prose. Use only the input fields below; invent no fields, functions, packages or data. Fields marked ? may be absent.

Goal: ${desiredOutcome.trim() || "<describe the behavior>"}

Output: ${outputContract}

Input (path: field type):
${compactInputContract(inputContract)}`;
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
    if (!(await copyText(prompt))) {
      toast.error(t("status.copyFailed"));
      return;
    }
    setCopied(true);
    if (copyResetTimerRef.current) {
      clearTimeout(copyResetTimerRef.current);
    }
    copyResetTimerRef.current = setTimeout(() => setCopied(false), 1500);
  }, [prompt, t]);

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
            className="bg-field text-foreground"
            rows={3}
          />
        </div>
        <div className="space-y-2">
          <Label id={`${id}-prompt-label`}>{t("settings.ruleAiPrompt")}</Label>
          <div className="relative">
            <pre
              id={`${id}-prompt`}
              aria-labelledby={`${id}-prompt-label`}
              className="border-border bg-background text-foreground max-h-[min(50dvh,28rem)] w-full overflow-auto rounded-md border py-2 pr-14 pl-3 font-[family-name:var(--font-code)] text-xs leading-5 break-words whitespace-pre-wrap"
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
              className="absolute top-2 right-2 border-border bg-background text-muted-foreground hover:bg-accent hover:text-foreground"
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
