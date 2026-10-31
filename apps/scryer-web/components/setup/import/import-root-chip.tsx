import { useRef, type CSSProperties, type DragEvent } from "react";
import {
  ArrowLeftRight,
  ArrowRight,
  FolderInput,
  FolderOpen,
  GripVertical,
  X,
} from "lucide-react";

import { IconButton } from "@/components/ui/icon-button";
import {
  effectiveRootPath,
  isRootRemapped,
  type ImportRoot,
} from "@/lib/hooks/use-external-import-setup";

import { ImportInstancePill } from "./import-instance-pill";

interface ImportRootChipProps {
  root: ImportRoot;
  variant: "tray" | "library";
  invalid?: boolean;
  onRemap?: () => void;
  onAssign?: () => void;
  onRemoveManual?: () => void;
  onBrowseManual?: () => void;
  draggable?: boolean;
  onDragStart?: (e: DragEvent) => void;
  onDragEnd?: (e: DragEvent) => void;
  t: (key: string, values?: Record<string, unknown>) => string;
}

const MONO: CSSProperties = {
  fontFamily: "var(--font-code)",
};

/**
 * Draggable source-root chip. Two variants:
 *  - "tray": the Source Roots tray chip (taller, full provenance display).
 *  - "library": the compact chip shown inside a library card's drop body.
 * Detected roots show their effective Scryer path (struck-through provenance
 * when remapped) and a Remap control; manual roots show an editable path input
 * and a delete button (no Remap).
 */
export function ImportRootChip({
  root,
  variant,
  invalid,
  onRemap,
  onAssign,
  onRemoveManual,
  onBrowseManual,
  draggable,
  onDragStart,
  onDragEnd,
  t,
}: ImportRootChipProps) {
  const library = variant === "library";
  const remapped = isRootRemapped(root);
  const effective = effectiveRootPath(root);
  const pathTitle = remapped
    ? `${t("setup.provenanceSource")}: ${root.arrRootPath}  →  ${t("setup.provenanceScryer")}: ${effective}`
    : effective;
  const blockDragRef = useRef(false);

  return (
    <span
      data-rootchip
      data-slot="import-root-chip"
      data-root-kind={root.kind}
      data-root-path={effective}
      data-root-variant={variant}
      aria-label={`${root.instanceLabel} ${effective}`}
      draggable={draggable}
      onPointerDownCapture={(e) => {
        blockDragRef.current = Boolean(
          (e.target as HTMLElement).closest(
            "button, input, textarea, select, a, [role='button']",
          ),
        );
      }}
      onPointerUpCapture={() => {
        blockDragRef.current = false;
      }}
      onPointerCancelCapture={() => {
        blockDragRef.current = false;
      }}
      onDragStart={(e) => {
        if (blockDragRef.current) {
          e.preventDefault();
          return;
        }
        onDragStart?.(e);
      }}
      onDragEnd={(e) => {
        blockDragRef.current = false;
        onDragEnd?.(e);
      }}
      style={{
        display: "inline-flex",
        alignItems: "center",
        gap: library ? 7 : 8,
        height: library ? 36 : 40,
        padding: "0 8px 0 2px",
        borderRadius: library ? 10 : 11,
        background: "var(--scry-bg)",
        border: "1px solid var(--scry-border2)",
        maxWidth: "100%",
        cursor: draggable ? "grab" : undefined,
        userSelect: "none",
      }}
    >
      {/* The grip is a visual cue; the full non-interactive chip surface drags. */}
      {draggable ? (
        <span
          data-root-drag-handle
          data-root-kind={root.kind}
          data-root-path={effective}
          data-root-variant={variant}
          aria-hidden
          style={{
            display: "inline-flex",
            alignItems: "center",
            justifyContent: "center",
            width: library ? 20 : 22,
            alignSelf: "stretch",
            cursor: draggable ? "grab" : "default",
            color: "var(--scry-faint3)",
            flex: "none",
          }}
        >
          <GripVertical size={library ? 13 : 14} />
        </span>
      ) : null}

      {/* Click-to-assign remains available even when native dragging is enabled. */}
      <IconButton
        type="button"
        label={
          library ? t("setup.moveToAnotherLibrary") : t("setup.assignToLibrary")
        }
        onClick={onAssign}
        tone="accent"
        className={
          library
            ? "h-[30px] w-[30px] flex-none rounded-[8px]"
            : "h-8 w-8 flex-none rounded-[8px]"
        }
      >
        <FolderInput className={library ? "h-[15px] w-[15px]" : "h-4 w-4"} />
      </IconButton>

      {/* Instance pill */}
      <ImportInstancePill
        kind={root.kind}
        label={root.instanceLabel}
        title={root.instanceLabel}
        size={library ? "sm" : "md"}
        showDot={root.kind === "manual"}
      />

      {/* Path region */}
      {root.manual ? (
        <ManualPathRegion
          value={root.arrRootPath}
          invalid={invalid}
          onBrowse={onBrowseManual}
          onRemove={onRemoveManual}
          t={t}
        />
      ) : library ? (
        <span
          title={pathTitle}
          style={{
            display: "inline-flex",
            alignItems: "center",
            gap: 4,
            flex: 1,
            minWidth: 0,
          }}
        >
          {remapped ? (
            <ArrowLeftRight
              size={11}
              style={{ color: "var(--scry-accent-text)", flex: "none" }}
            />
          ) : null}
          <span
            style={{
              ...MONO,
              fontSize: 12,
              color: remapped ? "var(--scry-accent-text)" : "var(--scry-ink2)",
              overflow: "hidden",
              textOverflow: "ellipsis",
              whiteSpace: "nowrap",
            }}
          >
            {effective}
          </span>
        </span>
      ) : (
        <span
          title={pathTitle}
          style={{
            display: "inline-flex",
            alignItems: "center",
            gap: 5,
            minWidth: 0,
          }}
        >
          {remapped ? (
            <>
              <span
                style={{
                  ...MONO,
                  fontSize: 11.5,
                  color: "var(--scry-faint3)",
                  textDecoration: "line-through",
                  textDecorationColor: "var(--scry-faint4)",
                  whiteSpace: "nowrap",
                }}
              >
                {root.arrRootPath}
              </span>
              <ArrowRight
                size={12}
                style={{ color: "var(--scry-accent-text)", flex: "none" }}
              />
              <span
                style={{
                  ...MONO,
                  fontSize: 13,
                  color: "var(--scry-accent-text)",
                  whiteSpace: "nowrap",
                }}
              >
                {effective}
              </span>
            </>
          ) : (
            <span
              style={{
                ...MONO,
                fontSize: 13,
                color: "var(--scry-ink2)",
                whiteSpace: "nowrap",
              }}
            >
              {effective}
            </span>
          )}
        </span>
      )}

      {/* Remap button — detected roots only */}
      {!root.manual ? (
        <button
          type="button"
          title={t("setup.remap")}
          onClick={onRemap}
          style={{
            display: "inline-flex",
            alignItems: "center",
            justifyContent: "center",
            height: library ? 28 : 26,
            padding: library ? "0 11px" : "0 10px",
            borderRadius: library ? 8 : 7,
            fontSize: 12,
            fontWeight: 600,
            cursor: "pointer",
            flex: "none",
            background: invalid
              ? "var(--scry-danger-bg-strong)"
              : remapped
                ? "rgba(var(--scry-accent-rgb), 0.1)"
                : "transparent",
            border: `1px solid ${
              invalid
                ? "var(--scry-danger-border-strong)"
                : remapped
                  ? "var(--scry-baccent)"
                  : "var(--scry-border2)"
            }`,
            color: invalid
              ? "var(--scry-danger-text-soft)"
              : remapped
                ? "var(--scry-accent-text)"
                : "var(--scry-faint)",
          }}
        >
          {t("setup.remap")}
        </button>
      ) : null}
    </span>
  );
}

function ManualPathRegion({
  value,
  invalid,
  onBrowse,
  onRemove,
  t,
}: {
  value: string;
  invalid?: boolean;
  onBrowse?: () => void;
  onRemove?: () => void;
  t: (key: string, values?: Record<string, unknown>) => string;
}) {
  return (
    <>
      {/* Folder-browser trigger — manual roots are chosen, never typed. */}
      <button
        type="button"
        onClick={onBrowse}
        aria-label={t("setup.manualRootPathAria")}
        aria-invalid={invalid || undefined}
        title={value || t("setup.chooseFolder")}
        style={{
          ...MONO,
          display: "inline-flex",
          alignItems: "center",
          gap: 6,
          maxWidth: 220,
          height: 28,
          padding: "0 9px",
          borderRadius: 7,
          border: `1px solid ${invalid ? "var(--scry-baccent)" : "var(--scry-border2)"}`,
          background: "var(--scry-page2)",
          color: value ? "var(--scry-ink2)" : "var(--scry-faint)",
          fontSize: 12.5,
          cursor: "pointer",
        }}
      >
        <FolderOpen
          size={13}
          style={{ flex: "none", color: "var(--scry-faint)" }}
        />
        <span
          style={{
            overflow: "hidden",
            textOverflow: "ellipsis",
            whiteSpace: "nowrap",
          }}
        >
          {value || t("setup.chooseFolder")}
        </span>
      </button>
      <IconButton
        type="button"
        label={t("setup.removeSourceRoot")}
        onClick={onRemove}
        appearance="ghost"
        tone="delete"
        className="h-6 w-6 flex-none rounded-[7px] text-[var(--scry-faint)] hover:text-[var(--scry-danger-text-soft)]"
      >
        <X className="h-3.5 w-3.5" />
      </IconButton>
    </>
  );
}
