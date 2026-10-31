import { useRef, type ComponentProps, type ReactNode } from "react";
import { Dialog as DialogPrimitive } from "radix-ui";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";

type ConfirmDialogProps = {
  open: boolean;
  title: string;
  description: string;
  confirmLabel: string;
  cancelLabel: string;
  contentClassName?: string;
  contentId?: string;
  confirmButtonId?: string;
  cancelButtonId?: string;
  confirmButtonVariant?: ComponentProps<typeof Button>["variant"];
  confirmButtonClassName?: string;
  isBusy?: boolean;
  confirmDisabled?: boolean;
  children?: ReactNode;
  onConfirm: () => Promise<void> | void;
  onCancel: () => void;
};

// Built on the Radix dialog so a confirmation opened from inside another dialog
// joins its layer stack: it renders above it, receives pointer input and focus,
// and Escape closes only the confirmation.
export function ConfirmDialog({
  open,
  title,
  description,
  confirmLabel,
  cancelLabel,
  contentClassName,
  contentId,
  confirmButtonId,
  cancelButtonId,
  confirmButtonVariant = "destructive",
  confirmButtonClassName,
  isBusy = false,
  confirmDisabled = false,
  children,
  onConfirm,
  onCancel,
}: ConfirmDialogProps) {
  const contentRef = useRef<HTMLElement>(null);
  const restoreFocusRef = useRef<HTMLElement | null>(null);

  if (!open) {
    return null;
  }

  return (
    <DialogPrimitive.Root
      open
      onOpenChange={(nextOpen) => {
        if (!nextOpen) {
          onCancel();
        }
      }}
    >
      <DialogPrimitive.Portal>
        <DialogPrimitive.Overlay className="fixed inset-0 z-[80] flex items-center justify-center bg-black/50 px-4">
          <DialogPrimitive.Content
            asChild
            onOpenAutoFocus={(event) => {
              // Keep focus off the confirm button so a stray Enter cannot confirm.
              event.preventDefault();
              restoreFocusRef.current =
                document.activeElement instanceof HTMLElement
                  ? document.activeElement
                  : null;
              contentRef.current?.focus();
            }}
            onCloseAutoFocus={(event) => {
              event.preventDefault();
              const restoreTarget = restoreFocusRef.current;
              restoreFocusRef.current = null;
              if (restoreTarget?.isConnected) {
                restoreTarget.focus();
              }
            }}
            onPointerDownOutside={(event) => event.preventDefault()}
            onInteractOutside={(event) => event.preventDefault()}
          >
            <section
              ref={contentRef}
              id={contentId}
              aria-modal="true"
              aria-label={title}
              {...(description ? {} : { "aria-describedby": undefined })}
              className={cn(
                "flex max-h-[calc(100dvh-2rem)] w-full max-w-md flex-col overflow-hidden rounded-lg border border-border bg-card p-4 shadow-lg outline-none",
                contentClassName,
              )}
            >
              <DialogPrimitive.Title className="mb-2 shrink-0 text-sm font-semibold">
                {title}
              </DialogPrimitive.Title>
              {description ? (
                <DialogPrimitive.Description className="mb-3 shrink-0 text-xs text-muted-foreground">
                  {description}
                </DialogPrimitive.Description>
              ) : null}
              {children ? (
                <div className="mb-4 min-h-0 overflow-y-auto overscroll-contain">
                  {children}
                </div>
              ) : null}
              <div className="flex shrink-0 justify-end gap-2">
                <Button
                  id={cancelButtonId}
                  type="button"
                  variant="secondary"
                  onClick={onCancel}
                  disabled={isBusy}
                >
                  {cancelLabel}
                </Button>
                <Button
                  id={confirmButtonId}
                  type="button"
                  variant={confirmButtonVariant}
                  className={confirmButtonClassName}
                  onClick={onConfirm}
                  disabled={isBusy || confirmDisabled}
                >
                  {confirmLabel}
                </Button>
              </div>
            </section>
          </DialogPrimitive.Content>
        </DialogPrimitive.Overlay>
      </DialogPrimitive.Portal>
    </DialogPrimitive.Root>
  );
}
