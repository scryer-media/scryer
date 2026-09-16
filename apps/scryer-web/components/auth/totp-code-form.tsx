import { Input, integerInputProps, sanitizeDigits } from "@/components/ui/input";
import { cn } from "@/lib/utils";
import { LoadingMark } from "@/components/common/loading-mark";

export function sanitizeTotpCode(value: string): string {
  return sanitizeDigits(value).slice(0, 6);
}

type TotpCodeFormProps = {
  id?: string;
  code: string;
  title: string;
  description: string;
  submitLabel: string;
  busyLabel?: string;
  cancelLabel?: string;
  inputId?: string;
  submitId?: string;
  cancelId?: string;
  busy?: boolean;
  disabled?: boolean;
  autoFocus?: boolean;
  allowRecoveryCode?: boolean;
  className?: string;
  onCodeChange: (code: string) => void;
  onSubmit: () => void | Promise<void>;
  onCancel?: () => void;
};

export function TotpCodeForm({
  id,
  code,
  title,
  description,
  submitLabel,
  busyLabel,
  cancelLabel,
  inputId,
  submitId,
  cancelId,
  busy = false,
  disabled = false,
  autoFocus = true,
  allowRecoveryCode = false,
  className,
  onCodeChange,
  onSubmit,
  onCancel,
}: TotpCodeFormProps) {
  const submitDisabled = disabled || busy || (allowRecoveryCode ? !code.trim() : code.length !== 6);

  return (
    <form
      id={id}
      onSubmit={(event) => {
        event.preventDefault();
        if (!submitDisabled) {
          void onSubmit();
        }
      }}
      className={cn("space-y-4", className)}
    >
      <div className="space-y-1 text-center">
        <h2 className="text-base font-semibold text-[var(--scry-ink)]">{title}</h2>
        <p className="text-sm leading-6 text-[var(--scry-muted)]">{description}</p>
      </div>
      <Input
        {...(allowRecoveryCode ? {} : integerInputProps)}
        id={inputId}
        autoComplete="one-time-code"
        autoFocus={autoFocus}
        maxLength={allowRecoveryCode ? 128 : 6}
        value={code}
        onChange={(event) =>
          onCodeChange(
            allowRecoveryCode ? event.target.value : sanitizeTotpCode(event.target.value),
          )
        }
        placeholder={title}
        className="h-10 rounded-[9px] border-[var(--scry-border3)] bg-[var(--scry-inset)] text-[var(--scry-ink2)] placeholder:text-[var(--scry-muted3)] focus-visible:border-[var(--scry-accent-ring)] focus-visible:ring-[rgba(var(--scry-accent-rgb),0.25)]"
      />
      <button
        id={submitId}
        type="submit"
        disabled={submitDisabled}
        className="flex h-10 w-full items-center justify-center gap-2 rounded-[9px] bg-primary px-4 text-sm font-semibold text-primary-foreground shadow-none transition-colors hover:bg-primary/90 disabled:opacity-50"
      >
        {busy ? <LoadingMark className="h-4 w-4" /> : null}
        {busy && busyLabel ? busyLabel : submitLabel}
      </button>
      {onCancel && cancelLabel ? (
        <button
          id={cancelId}
          type="button"
          onClick={onCancel}
          disabled={disabled || busy}
          className="h-10 w-full rounded-[9px] border border-[var(--scry-border2)] bg-[var(--scry-inset)] px-4 text-sm font-semibold text-[var(--scry-ink2)] shadow-none transition-colors hover:bg-[var(--scry-hover)] disabled:opacity-50"
        >
          {cancelLabel}
        </button>
      ) : null}
    </form>
  );
}
