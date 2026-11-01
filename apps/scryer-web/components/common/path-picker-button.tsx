import * as React from "react";
import { FolderOpen } from "lucide-react";

import { FolderBrowserDialog } from "@/components/setup/folder-browser-dialog";
import { Button } from "@/components/ui/button";

type PathPickerButtonProps = {
  id: string;
  value: string;
  /** Shown while no path is picked. */
  placeholder: string;
  onSelect: (path: string) => void;
  selectionTypes: Array<"folder" | "file">;
  dialogTitle: string;
  /** Where the picker opens; defaults to the picked path. */
  initialPath?: string;
  disabled?: boolean;
};

/** A path picked from the Scryer host through the folder browser, never typed. */
export function PathPickerButton({
  id,
  value,
  placeholder,
  onSelect,
  selectionTypes,
  dialogTitle,
  initialPath,
  disabled = false,
}: PathPickerButtonProps) {
  const [open, setOpen] = React.useState(false);
  return (
    <>
      <Button
        id={id}
        type="button"
        variant="outline"
        disabled={disabled}
        aria-haspopup="dialog"
        onClick={() => setOpen(true)}
        className="min-w-0 flex-1 justify-start font-[family-name:var(--font-code)]"
        title={value || placeholder}
        data-value={value}
      >
        <FolderOpen className="mr-2 h-4 w-4 shrink-0" />
        <span className="truncate">{value || placeholder}</span>
      </Button>
      {open ? (
        <FolderBrowserDialog
          open={open}
          onOpenChange={setOpen}
          onSelect={onSelect}
          selectionTypes={selectionTypes}
          initialPath={initialPath ?? (value || "/")}
          title={dialogTitle}
        />
      ) : null}
    </>
  );
}
