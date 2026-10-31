import * as React from "react";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { cn } from "@/lib/utils";

/**
 * Interactive search results open in this dialog rather than inline, so
 * they get the full width of the screen without squeezing the title list
 * and overview panels. It fills the screen on phones.
 */
export function InteractiveSearchDialog({
  id,
  open,
  onOpenChange,
  title,
  description,
  children,
}: {
  id?: string;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  title: string;
  description?: string;
  children: React.ReactNode;
}) {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent
        id={id}
        className={cn(
          "flex h-[100dvh] max-h-[100dvh] max-w-none flex-col gap-3 overflow-hidden rounded-none border-0 sm:h-[88vh] sm:max-h-[88vh] sm:w-[calc(100%-2rem)] sm:max-w-6xl sm:rounded-lg sm:border sm:p-5",
          // Full screen on phones, so the header and close button clear the
          // status bar and notch.
          "pt-[calc(env(safe-area-inset-top)+0.75rem)] pr-[calc(env(safe-area-inset-right)+0.75rem)] pb-[calc(env(safe-area-inset-bottom)+0.75rem)] pl-[calc(env(safe-area-inset-left)+0.75rem)]",
          "[&>[data-slot=dialog-close]]:top-[calc(env(safe-area-inset-top)+1rem)] [&>[data-slot=dialog-close]]:right-[calc(env(safe-area-inset-right)+1rem)] sm:[&>[data-slot=dialog-close]]:top-4 sm:[&>[data-slot=dialog-close]]:right-4",
        )}
      >
        <DialogHeader className="pr-8 text-left">
          <DialogTitle className="truncate">{title}</DialogTitle>
          {description ? (
            <DialogDescription className="truncate">
              {description}
            </DialogDescription>
          ) : null}
        </DialogHeader>
        <div className="min-h-0 flex-1 overflow-y-auto">{children}</div>
      </DialogContent>
    </Dialog>
  );
}
