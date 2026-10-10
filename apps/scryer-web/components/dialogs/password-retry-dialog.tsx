import { useRef, useState } from "react";
import { ConfirmDialog } from "@/components/common/confirm-dialog";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { useTranslate } from "@/lib/context/translate-context";

export function PasswordRetryDialog({ open, jobLabel, onConfirm, onCancel, confirmLabel }: {
  open: boolean;
  jobLabel: string;
  onConfirm: (password: string) => Promise<void>;
  onCancel: () => void;
  confirmLabel?: string;
}) {
  const t = useTranslate();
  const [password, setPassword] = useState("");
  const [revealed, setRevealed] = useState(false);
  const [isBusy, setBusy] = useState(false);
  const submitting = useRef(false);
  const clear = () => { setPassword(""); setRevealed(false); };
  return (
    <ConfirmDialog
      open={open}
      title={t("importHistory.retryWithPassword")}
      description={t("importHistory.passwordRequired")}
      contentId="archive-password-retry-dialog"
      confirmButtonId="archive-password-retry-confirm"
      cancelButtonId="archive-password-retry-cancel"
      confirmLabel={confirmLabel ?? t("importHistory.retry")}
      cancelLabel={t("label.cancel")}
      confirmButtonVariant="default"
      confirmDisabled={password.length === 0}
      isBusy={isBusy}
      onCancel={() => { if (!submitting.current) { clear(); onCancel(); } }}
      onConfirm={async () => {
        if (password.length === 0 || submitting.current) return;
        submitting.current = true;
        setBusy(true);
        const replacement = password;
        clear();
        try { await onConfirm(replacement); }
        finally { submitting.current = false; setBusy(false); }
      }}
    >
      <p className="mb-3 break-all text-sm">{jobLabel}</p>
      <div className="flex gap-2">
        <Input
          id="archive-password-retry-input"
          type={revealed ? "text" : "password"}
          value={password}
          disabled={isBusy}
          onChange={(event) => setPassword(event.target.value)}
          aria-label={t("importHistory.passwordPlaceholder")}
          autoComplete="new-password"
        />
        <Button type="button" variant="outline" disabled={isBusy} onClick={() => setRevealed(!revealed)}>
          {t(revealed ? "importHistory.hidePassword" : "importHistory.showPassword")}
        </Button>
      </div>
    </ConfirmDialog>
  );
}
