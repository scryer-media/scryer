import * as React from "react";
import { FolderSymlink } from "lucide-react";

import { Button } from "@/components/ui/button";
import {
  ChangeTitleFolderDialog,
  type ChangeFolderTitle,
} from "@/components/dialogs/change-title-folder-dialog";
import {
  folderMatchOutcomeMessage,
  type ChangeFolderResult,
} from "@/lib/change-title-folder";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { useTranslate } from "@/lib/context/translate-context";
import type { LibraryRootRecord } from "@/lib/types/titles";

type ChangeTitleFolderCardProps = {
  title: ChangeFolderTitle;
  roots: LibraryRootRecord[];
  idPrefix: string;
  onTitleChanged?: () => Promise<void> | void;
};

/**
 * Folder-match correction entry point: reassigns which existing folder a title
 * owns and rescans it. It never moves file content.
 */
export function ChangeTitleFolderCard({
  title,
  roots,
  idPrefix,
  onTitleChanged,
}: ChangeTitleFolderCardProps) {
  const t = useTranslate();
  const setGlobalStatus = useGlobalStatus();
  const [open, setOpen] = React.useState(false);

  const handleChanged = React.useCallback(
    async (result: ChangeFolderResult) => {
      setGlobalStatus(folderMatchOutcomeMessage(result, t));
      await onTitleChanged?.();
    },
    [onTitleChanged, setGlobalStatus, t],
  );

  return (
    <>
      <div className="mt-3 flex items-center justify-between gap-3 rounded-lg border border-border/70 bg-muted/20 px-3 py-3">
        <div className="min-w-0">
          <p className="text-sm font-medium text-foreground">
            {t("title.changeFolderHeading")}
          </p>
          <p className="text-xs text-muted-foreground">
            {t("title.changeFolderDescription")}
          </p>
        </div>
        <Button
          id={`${idPrefix}-change-folder`}
          type="button"
          variant="primary"
          size="sm"
          className="shrink-0"
          onClick={() => setOpen(true)}
        >
          <FolderSymlink className="mr-2 h-4 w-4" />
          {t("title.changeFolderAction")}
        </Button>
      </div>
      <ChangeTitleFolderDialog
        open={open}
        onOpenChange={setOpen}
        title={title}
        roots={roots}
        onChanged={handleChanged}
      />
    </>
  );
}
