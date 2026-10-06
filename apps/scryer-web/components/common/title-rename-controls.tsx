import { Eye } from "lucide-react";

import { LoadingMark } from "@/components/common/loading-mark";
import {
  MediaRenamePlanPanel,
  renamePlanHasWork,
} from "@/components/common/media-rename-plan-panel";
import { Button } from "@/components/ui/button";
import { useTranslate } from "@/lib/context/translate-context";
import type { TitleRenameController } from "@/lib/hooks/use-title-rename";

/**
 * The rename preview button both title overviews show. It is absent whenever
 * the title cannot be renamed.
 */
export function TitleRenamePreviewButton({
  rename,
  id,
  dataUi,
  titleId,
  className,
}: {
  rename: TitleRenameController;
  id: string;
  dataUi: string;
  titleId?: string;
  className?: string;
}) {
  const t = useTranslate();
  if (!rename.available) {
    return null;
  }
  return (
    <Button
      id={id}
      data-ui={dataUi}
      data-title-id={titleId}
      type="button"
      variant="primary"
      size="sm"
      className={className}
      onClick={() => void rename.preview()}
      disabled={rename.previewing || rename.applying}
    >
      {rename.previewing ? (
        <LoadingMark className="h-4 w-4" />
      ) : (
        <Eye className="h-4 w-4" />
      )}
      <span>
        {rename.previewing ? t("rename.previewing") : t("rename.previewButton")}
      </span>
    </Button>
  );
}

/** The previewed rename, with the button that applies it. */
export function TitleRenamePlan({
  rename,
  applyButtonId,
}: {
  rename: TitleRenameController;
  applyButtonId: string;
}) {
  if (!rename.plan) {
    return null;
  }
  return (
    <MediaRenamePlanPanel
      plan={rename.plan}
      applying={rename.applying}
      applyDisabled={
        rename.applying ||
        rename.previewing ||
        !renamePlanHasWork(rename.plan)
      }
      applyButtonId={applyButtonId}
      onApply={() => void rename.apply()}
      onCancel={rename.cancel}
    />
  );
}
