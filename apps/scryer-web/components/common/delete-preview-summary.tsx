import { Input } from "@/components/ui/input";
import { useTranslate } from "@/lib/context/translate-context";
import type { DeletePreview } from "@/lib/types/delete-preview";

type DeletePreviewSummaryProps = {
  preview: DeletePreview | null;
  loading: boolean;
  error: string | null;
  typedConfirmation: string;
  onTypedConfirmationChange: (value: string) => void;
};

export function DeletePreviewSummary({
  preview,
  loading,
  error,
  typedConfirmation,
  onTypedConfirmationChange,
}: DeletePreviewSummaryProps) {
  const t = useTranslate();

  if (loading) {
    return (
      <p className="rounded border border-border bg-muted/40 px-3 py-2 text-xs text-muted-foreground">
        {t("deletePreview.counting")}
      </p>
    );
  }

  if (error) {
    return (
      <div className="space-y-2 rounded border border-destructive/40 bg-destructive/10 px-3 py-2">
        <p className="text-xs font-medium text-destructive">
          {t("deletePreview.error")}
        </p>
        <p className="text-xs text-destructive/80">{error}</p>
      </div>
    );
  }

  if (!preview) {
    return null;
  }

  return (
    <div className="space-y-3 rounded border border-border bg-muted/20 p-3">
      <div className="grid grid-cols-2 gap-2 text-xs sm:grid-cols-3">
        <PreviewCount label={t("deletePreview.files")} value={preview.totalFileCount} />
        <PreviewCount label={t("deletePreview.media")} value={preview.mediaCount} />
        <PreviewCount
          label={t("deletePreview.subtitles")}
          value={preview.subtitleCount}
        />
        <PreviewCount label={t("deletePreview.images")} value={preview.imageCount} />
        <PreviewCount label={t("deletePreview.other")} value={preview.otherCount} />
        <PreviewCount label={t("deletePreview.folders")} value={preview.directoryCount} />
      </div>
      {preview.samplePaths.length > 0 ? (
        <div className="space-y-1">
          <p className="text-[11px] font-medium uppercase tracking-wide text-muted-foreground">
            {t("deletePreview.samplePaths")}
          </p>
          <div className="space-y-1">
            {preview.samplePaths.map((path) => (
              <p key={path} className="break-all font-mono text-[11px] text-muted-foreground">
                {path}
              </p>
            ))}
          </div>
        </div>
      ) : null}
      {preview.requiresTypedConfirmation ? (
        <div className="space-y-2">
          <p className="text-xs font-medium text-foreground">
            {preview.typedConfirmationPrompt ?? t("deletePreview.confirmPrompt")}
          </p>
          <Input
            value={typedConfirmation}
            onChange={(event) => onTypedConfirmationChange(event.target.value)}
            placeholder="DELETE"
            autoCapitalize="characters"
            autoCorrect="off"
            spellCheck={false}
          />
        </div>
      ) : null}
    </div>
  );
}

function PreviewCount({ label, value }: { label: string; value: number }) {
  return (
    <div className="rounded border border-border bg-background/80 px-2 py-1.5">
      <p className="text-[11px] uppercase tracking-wide text-muted-foreground">{label}</p>
      <p className="text-sm font-semibold">{value}</p>
    </div>
  );
}
