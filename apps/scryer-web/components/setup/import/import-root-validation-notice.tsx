import { TriangleAlert } from "lucide-react";
import { LoadingMark } from "@/components/common/loading-mark";

export interface InvalidMappedRootNoticeItem {
  id: string;
  name: string;
  path: string;
}

interface ImportRootValidationNoticeProps {
  checking: boolean;
  invalidRoots: readonly InvalidMappedRootNoticeItem[];
  onRemap: (rootId: string) => void;
  t: (key: string, values?: Record<string, unknown>) => string;
}

export function ImportRootValidationNotice({
  checking,
  invalidRoots,
  onRemap,
  t,
}: ImportRootValidationNoticeProps) {
  if (checking) {
    return (
      <div
        data-slot="mapped-root-validation-loading"
        role="status"
        aria-live="polite"
        className="mt-3.5 flex items-center gap-2.5 rounded-[11px] border border-[var(--scry-border2)] bg-[rgba(10,17,32,0.5)] px-3.5 py-3 text-[12.5px] text-[var(--scry-muted2)]"
      >
        <LoadingMark className="size-[16px]" />
        {t("setup.mappedPathValidationChecking")}
      </div>
    );
  }

  if (invalidRoots.length === 0) return null;

  return (
    <div
      data-slot="invalid-mapped-root-notice"
      role="alert"
      className="mt-3.5 flex items-start gap-[11px] rounded-[11px] border border-[var(--scry-warning-border)] bg-[var(--scry-warning-bg)] px-3.5 py-[13px] text-[var(--scry-warning-text)]"
    >
      <TriangleAlert size={18} aria-hidden className="mt-px shrink-0" />
      <div className="min-w-0 flex-1">
        <div className="text-[13px] font-bold">
          {invalidRoots.length === 1
            ? t("setup.invalidMappedPathTitle")
            : t("setup.invalidMappedPathsTitle", {
                count: invalidRoots.length,
              })}
        </div>
        <div className="mt-[3px] text-[12.5px] leading-[1.45]">
          {t("setup.invalidMappedPathsHelp")}
        </div>
        <div className="mt-2.5 flex flex-wrap gap-2">
          {invalidRoots.map((root) => (
            <button
              key={root.id}
              type="button"
              onClick={() => onRemap(root.id)}
              aria-label={t("setup.remapInvalidRootAria", {
                name: root.name,
                path: root.path,
              })}
              className="inline-flex h-8 min-w-0 max-w-full cursor-pointer items-center gap-2 rounded-lg border border-[var(--scry-warning-border)] bg-[var(--scry-bg)] px-2.5 text-xs text-[var(--scry-warning-text)]"
            >
              <strong className="shrink-0">{root.name}</strong>
              <span
                title={root.path}
                className="overflow-hidden text-ellipsis whitespace-nowrap font-mono text-[var(--scry-muted2)]"
              >
                {root.path}
              </span>
              <span className="shrink-0 font-bold">{t("setup.remap")}</span>
            </button>
          ))}
        </div>
      </div>
    </div>
  );
}
