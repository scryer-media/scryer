import type { CSSProperties } from "react";
import { Check, ChevronRight, FolderMinus, Library, Plus } from "lucide-react";

import {
  SheetContent,
  Sheet,
  SheetHeader,
  SheetTitle,
} from "@/components/ui/sheet";
import {
  effectiveRootPath,
  facetsForKind,
  kindCompatibleWithFacet,
  type ImportLibraryDraft,
  type ImportRoot,
  type WizardFacet,
} from "@/lib/hooks/use-external-import-setup";
import { facetLabelKey, facetPillStyle, facetStyle } from "./facet-style";

import { ImportInstancePill } from "./import-instance-pill";

interface ImportAssignSheetProps {
  root: ImportRoot | null;
  libraries: ImportLibraryDraft[];
  currentLibraryId: string | null;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onPick: (libraryId: string | null) => void;
  onCreateLibrary?: (facet: WizardFacet) => void;
  t: (key: string, values?: Record<string, unknown>) => string;
}

const MONO: CSSProperties = {
  fontFamily: "var(--font-code)",
};

/**
 * Mobile bottom sheet for assigning a source root to a library. Shows only the
 * libraries compatible with the root's kind, plus a "move to unassigned" row
 * when the root is currently mapped.
 */
export function ImportAssignSheet({
  root,
  libraries,
  currentLibraryId,
  open,
  onOpenChange,
  onPick,
  onCreateLibrary,
  t,
}: ImportAssignSheetProps) {
  const compatible = root
    ? libraries.filter((lib) => kindCompatibleWithFacet(root.kind, lib.facet))
    : [];

  return (
    <Sheet open={open} onOpenChange={onOpenChange}>
      <SheetContent
        side="bottom"
        className="gap-3 px-[18px] pt-[10px] pb-[calc(26px+env(safe-area-inset-bottom))]"
        style={{
          background: "var(--scry-page2)",
          border: "1px solid var(--scry-border)",
          borderBottom: "none",
          borderTopLeftRadius: 20,
          borderTopRightRadius: 20,
          maxHeight: "80vh",
          overflowY: "auto",
        }}
      >
        {/* Grab handle */}
        <span
          aria-hidden
          style={{
            width: 38,
            height: 4,
            borderRadius: 999,
            background: "var(--scry-border3)",
            margin: "6px auto 4px",
          }}
        />

        <SheetHeader className="p-0">
          <SheetTitle
            style={{
              fontFamily: "var(--font-space-grotesk)",
              fontSize: 17,
              fontWeight: 700,
              color: "#fff",
            }}
          >
            {t("setup.assignSheetTitle")}
          </SheetTitle>
        </SheetHeader>

        {/* Root preview */}
        {root ? (
          <div
            style={{
              display: "flex",
              alignItems: "center",
              gap: 9,
              padding: "9px 11px",
              borderRadius: 11,
              border: "1px solid var(--scry-border2)",
              background: "var(--scry-bg)",
            }}
          >
            <ImportInstancePill
              kind={root.kind}
              label={root.instanceLabel}
              size="sm"
              showDot={root.kind === "manual"}
            />
            <span
              style={{
                ...MONO,
                fontSize: 13,
                color: "var(--scry-ink2)",
                overflow: "hidden",
                textOverflow: "ellipsis",
                whiteSpace: "nowrap",
              }}
            >
              {effectiveRootPath(root) || t("setup.noPathSet")}
            </span>
          </div>
        ) : null}

        {/* Library rows */}
        <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
          {compatible.map((lib) => {
            const style = facetStyle(lib.facet);
            const isCurrent = lib.id === currentLibraryId;
            return (
              <button
                key={lib.id}
                type="button"
                onClick={() => {
                  onPick(lib.id);
                  onOpenChange(false);
                }}
                style={{
                  display: "flex",
                  alignItems: "center",
                  gap: 11,
                  minHeight: 52,
                  padding: "8px 14px",
                  borderRadius: 12,
                  textAlign: "left",
                  cursor: "pointer",
                  background: isCurrent ? style.bg : "var(--scry-bg)",
                  border: `1px solid ${
                    isCurrent ? style.border : "var(--scry-border2)"
                  }`,
                }}
              >
                <span
                  aria-hidden
                  style={{
                    display: "inline-flex",
                    alignItems: "center",
                    justifyContent: "center",
                    width: 34,
                    height: 34,
                    borderRadius: 8,
                    background: style.bg,
                    border: `1px solid ${style.border}`,
                    color: style.text,
                    flex: "none",
                  }}
                >
                  <Library size={17} />
                </span>
                <span
                  style={{
                    display: "flex",
                    flexDirection: "column",
                    gap: 2,
                    flex: 1,
                    minWidth: 0,
                  }}
                >
                  <span
                    style={{
                      fontSize: 14.5,
                      fontWeight: 600,
                      color: "#f1f5ff",
                      overflow: "hidden",
                      textOverflow: "ellipsis",
                      whiteSpace: "nowrap",
                    }}
                  >
                    {lib.name}
                  </span>
                  <span
                    style={{
                      display: "inline-flex",
                      alignItems: "center",
                      gap: 5,
                      fontSize: 11,
                      color: "var(--scry-faint)",
                    }}
                  >
                    <span
                      aria-hidden
                      style={{
                        width: 6,
                        height: 6,
                        borderRadius: "50%",
                        background: style.dot,
                      }}
                    />
                    {t(facetLabelKey(lib.facet))}
                  </span>
                </span>
                {isCurrent ? (
                  <Check
                    size={18}
                    style={{ color: style.text, flex: "none" }}
                  />
                ) : (
                  <ChevronRight
                    size={17}
                    style={{ color: "var(--scry-faint3)", flex: "none" }}
                  />
                )}
              </button>
            );
          })}

          {/* Empty state: no compatible library — offer to create one. */}
          {root && compatible.length === 0 ? (
            <div
              style={{
                display: "flex",
                flexDirection: "column",
                gap: 8,
              }}
            >
              <span
                style={{
                  fontSize: 13,
                  color: "var(--scry-faint)",
                  padding: "4px 2px",
                }}
              >
                {t("setup.noCompatibleLibrary")}
              </span>
              {facetsForKind(root.kind).map((facet) => {
                const facetName = facetStyle(facet);
                return (
                  <button
                    key={facet}
                    type="button"
                    onClick={() => {
                      onCreateLibrary?.(facet);
                      onOpenChange(false);
                    }}
                    style={{
                      display: "inline-flex",
                      alignItems: "center",
                      gap: 9,
                      minHeight: 48,
                      padding: "8px 14px",
                      borderRadius: 12,
                      textAlign: "left",
                      cursor: "pointer",
                      ...facetPillStyle(facet),
                    }}
                  >
                    <span
                      aria-hidden
                      style={{
                        display: "inline-flex",
                        alignItems: "center",
                        justifyContent: "center",
                        width: 30,
                        height: 30,
                        borderRadius: 8,
                        background: facetName.bg,
                        border: `1px solid ${facetName.border}`,
                        color: facetName.text,
                        flex: "none",
                      }}
                    >
                      <Plus size={16} />
                    </span>
                    <span style={{ fontSize: 14, fontWeight: 600 }}>
                      {t("setup.createFacetLibrary", {
                        facet: t(facetLabelKey(facet)),
                      })}
                    </span>
                  </button>
                );
              })}
            </div>
          ) : null}
        </div>

        {/* Move to unassigned */}
        {currentLibraryId ? (
          <button
            type="button"
            onClick={() => {
              onPick(null);
              onOpenChange(false);
            }}
            style={{
              display: "flex",
              alignItems: "center",
              gap: 11,
              minHeight: 48,
              padding: "8px 14px",
              borderRadius: 12,
              textAlign: "left",
              cursor: "pointer",
              background: "transparent",
              border: "1px dashed var(--scry-border2)",
              color: "var(--scry-muted2)",
            }}
          >
            <span
              aria-hidden
              style={{
                display: "inline-flex",
                alignItems: "center",
                justifyContent: "center",
                width: 34,
                height: 34,
                borderRadius: 8,
                border: "1px solid var(--scry-border2)",
                color: "var(--scry-faint)",
                flex: "none",
              }}
            >
              <FolderMinus size={16} />
            </span>
            <span style={{ fontSize: 14, fontWeight: 600 }}>
              {t("setup.moveToUnassigned")}
            </span>
          </button>
        ) : null}
      </SheetContent>
    </Sheet>
  );
}
