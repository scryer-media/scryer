import {
  regoDiagnosticMessage,
  type RuleValidationDiagnostic,
} from "@/lib/utils/rego-diagnostics";
import { useId, useRef, useState, type ComponentProps } from "react";
import { Check, X } from "lucide-react";
import { LoadingMark } from "@/components/common/loading-mark";
import { useTranslate } from "@/lib/context/translate-context";
import { cn } from "@/lib/utils";

type ValidationResult = {
  valid: boolean;
  errors: readonly string[];
  diagnostics?: readonly RuleValidationDiagnostic[];
};
type EditorValidationStatusProps = ComponentProps<"div"> & {
  checking: boolean;
  result: ValidationResult | null;
  formatError?: (error: string) => string;
};

export function EditorValidationStatus({
  checking,
  result,
  formatError,
  ...props
}: EditorValidationStatusProps) {
  if (!checking && !result) return null;
  return (
    <ValidationBadge
      key={JSON.stringify([checking, result])}
      checking={checking}
      result={result}
      formatError={formatError}
      {...props}
    />
  );
}

function ValidationBadge({
  checking,
  result,
  formatError,
  className,
  ...props
}: EditorValidationStatusProps) {
  const t = useTranslate();
  const errorId = useId();
  const [hovered, setHovered] = useState(false);
  const [focused, setFocused] = useState(false);
  const [pinned, setPinned] = useState(false);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const failed = !checking && result?.valid === false;
  const errorMessages = result?.diagnostics?.length
    ? result.diagnostics.map((diagnostic) =>
        regoDiagnosticMessage(diagnostic, t),
      )
    : result?.errors;
  const expanded = failed && (hovered || focused || pinned);
  const label = checking
    ? t("settings.ruleValidating")
    : failed
      ? t("settings.regoValidationFailed")
      : t("settings.ruleValid");

  return (
    <div
      {...props}
      data-editor-validation={
        checking ? "checking" : failed ? "failed" : "passed"
      }
      className={cn(
        "absolute right-2 top-2 z-20 flex max-h-[calc(100%-1rem)] max-w-[calc(100%-1rem)] flex-col rounded-md border bg-background shadow-md",
        expanded ? "w-[32rem]" : "w-9",
        checking
          ? "border-border text-muted-foreground"
          : failed
            ? "border-[var(--scry-danger-border)] text-[var(--scry-danger-text)]"
            : "border-[var(--scry-success-border)] text-[var(--scry-success-text)]",
        className,
      )}
      onPointerEnter={(event) => {
        if (event.pointerType !== "touch") setHovered(true);
      }}
      onPointerLeave={() => setHovered(false)}
      onFocusCapture={(event) => {
        if (event.target.matches(":focus-visible")) setFocused(true);
      }}
      onBlurCapture={(event) => {
        if (!event.currentTarget.contains(event.relatedTarget)) {
          setFocused(false);
          setPinned(false);
        }
      }}
      onKeyDown={(event) => {
        if (event.key === "Escape" && expanded) {
          event.preventDefault();
          event.stopPropagation();
          triggerRef.current?.focus();
          setHovered(false);
          setFocused(false);
          setPinned(false);
        }
      }}
    >
      <span role="status" aria-live="polite" className="sr-only">
        {label}
      </span>
      {failed ? (
        <button
          type="button"
          ref={triggerRef}
          aria-label={label}
          aria-expanded={expanded}
          aria-controls={expanded ? errorId : undefined}
          aria-describedby={expanded ? errorId : undefined}
          className="ml-auto flex h-8 w-[34px] shrink-0 items-center justify-center rounded-md focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring"
          onClick={() => setPinned((value) => !value)}
        >
          <X className="h-4 w-4" aria-hidden="true" />
        </button>
      ) : (
        <span
          className="flex h-8 w-[34px] shrink-0 items-center justify-center"
          aria-hidden="true"
        >
          {checking ? (
            <LoadingMark className="h-4 w-4" />
          ) : (
            <Check className="h-4 w-4" />
          )}
        </span>
      )}
      {expanded ? (
        <div
          id={errorId}
          role="region"
          aria-label={label}
          tabIndex={0}
          className="min-h-0 space-y-2 overflow-y-auto px-3 pb-3 text-xs"
        >
          {(errorMessages?.length ? errorMessages : [label]).map(
            (error, index) => (
              <pre
                key={index}
                className="whitespace-pre-wrap break-words font-[var(--font-code)] leading-5 [overflow-wrap:anywhere]"
              >
                {formatError ? formatError(error) : error}
              </pre>
            ),
          )}
        </div>
      ) : null}
    </div>
  );
}
