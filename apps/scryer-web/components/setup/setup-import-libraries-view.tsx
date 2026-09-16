import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
  type DragEvent,
} from "react";
import {
  ArrowDown,
  CheckCheck,
  FolderSymlink,
  Library,
  Plus,
  RotateCw,
  Trash2,
  TriangleAlert,
} from "lucide-react";

import { AddNewButton } from "@/components/common/add-new-button";
import {
  importLibraryDropState,
  shouldEnableNativeImportDrag,
} from "@/lib/external-import-library-drag";
import { useIsMobile } from "@/lib/hooks/use-mobile";
import {
  effectiveRootPath,
  kindCompatibleWithFacet,
  type ImportRoot,
  type UseExternalImportSetupReturn,
  type WizardFacet,
} from "@/lib/hooks/use-external-import-setup";

import { FolderBrowserDialog } from "./folder-browser-dialog";
import { ImportAssignSheet } from "./import/import-assign-sheet";
import { ImportRemapDialog } from "./import/import-remap-dialog";
import { ImportRootChip } from "./import/import-root-chip";
import { ImportRootValidationNotice } from "./import/import-root-validation-notice";
import {
  facetLabelKey,
  facetPillStyle,
  facetStyle,
} from "./import/facet-style";
import { Button } from "@/components/ui/button";
import { IconButton } from "@/components/ui/icon-button";
import { TextActionButton } from "@/components/ui/text-action-button";
import { selectorToken } from "@/lib/utils/dom-ids";
import { LoadingMark } from "@/components/common/loading-mark";

interface SetupImportLibrariesViewProps {
  wizard: UseExternalImportSetupReturn;
  t: (key: string, values?: Record<string, unknown>) => string;
}

const FACET_PICKER: WizardFacet[] = ["MOVIE", "SERIES", "ANIME"];

// Native drag-and-drop is reserved for wide, fine-pointer layouts. The
// click-to-place assign sheet remains available everywhere.
const MOBILE_BREAKPOINT = 1025;

function useHasCoarsePointer(): boolean {
  const [hasCoarsePointer, setHasCoarsePointer] = useState(() =>
    typeof window === "undefined"
      ? false
      : window.matchMedia("(pointer: coarse)").matches,
  );

  useEffect(() => {
    const query = window.matchMedia("(pointer: coarse)");
    const update = () => setHasCoarsePointer(query.matches);
    query.addEventListener("change", update);
    update();
    return () => query.removeEventListener("change", update);
  }, []);

  return hasCoarsePointer;
}

const SECTION_LABEL: CSSProperties = {
  fontSize: 11,
  fontWeight: 700,
  letterSpacing: "0.1em",
  textTransform: "uppercase",
  color: "var(--scry-faint2)",
};

const DROP_OUTLINE = "2px dashed var(--scry-accent)";

export default function SetupImportLibrariesView({
  wizard,
  t,
}: SetupImportLibrariesViewProps) {
  const {
    roots,
    trayRoots,
    previewing,
    previewError,
    loadPreview,
    libraries,
    rootsForLibrary,
    assign,
    assignRoot,
    addManualRoot,
    setManualRootPath,
    removeManualRoot,
    addLibrary,
    renameLibrary,
    removeLibrary,
    setRootRemap,
    invalidAssignedRootIds,
    assignedRootPathValidationLoading,
    rootById,
  } = wizard;

  const isMobile = useIsMobile(MOBILE_BREAKPOINT);
  const hasCoarsePointer = useHasCoarsePointer();
  const nativeDragEnabled = shouldEnableNativeImportDrag(
    isMobile,
    hasCoarsePointer,
  );
  const invalidAssignedRootIdSet = useMemo(
    () => new Set(invalidAssignedRootIds),
    [invalidAssignedRootIds],
  );
  const invalidAssignedRoots = useMemo(
    () =>
      invalidAssignedRootIds.flatMap((rootId) => {
        const root = rootById(rootId);
        if (!root) return [];
        const libraryName =
          libraries.find((library) => library.id === assign[root.id])?.name ??
          root.instanceLabel;
        return [
          {
            id: root.id,
            name: libraryName,
            path: effectiveRootPath(root),
          },
        ];
      }),
    [assign, invalidAssignedRootIds, libraries, rootById],
  );

  // ── Local UI state ─────────────────────────────────────────────────────────
  const [assignSheetRootId, setAssignSheetRootId] = useState<string | null>(
    null,
  );
  const [remapRootId, setRemapRootId] = useState<string | null>(null);
  const [addingLib, setAddingLib] = useState(false);
  const [editLibId, setEditLibId] = useState<string | null>(null);
  const [editLibVal, setEditLibVal] = useState("");
  // Folder-browser target: a new manual root, or an existing one being changed.
  const [browseTarget, setBrowseTarget] = useState<
    { kind: "new" } | { kind: "manual"; rootId: string } | null
  >(null);

  const handleBrowseSelect = (path: string) => {
    if (!browseTarget) return;
    if (browseTarget.kind === "new") addManualRoot(path);
    else setManualRootPath(browseTarget.rootId, path);
    setBrowseTarget(null);
  };

  const dragRootId = useRef<string | null>(null);
  const [draggingRootId, setDraggingRootId] = useState<string | null>(null);

  const assignSheetRoot =
    assignSheetRootId != null ? wizard.rootById(assignSheetRootId) : null;
  const remapRoot = remapRootId != null ? wizard.rootById(remapRootId) : null;

  // ── Desktop drag-and-drop (HTML5) ──────────────────────────────────────────
  const onDragStart = useCallback((e: DragEvent, rootId: string) => {
    dragRootId.current = rootId;
    setDraggingRootId(rootId);
    e.dataTransfer.effectAllowed = "move";
    e.dataTransfer.setData("text/plain", rootId);
    const chip = (e.target as HTMLElement).closest<HTMLElement>(
      "[data-rootchip]",
    );
    if (chip) {
      // Drag the whole chip box (not just the grip) as the drag image,
      // positioned so it stays under the cursor.
      const rect = chip.getBoundingClientRect();
      e.dataTransfer.setDragImage(
        chip,
        e.clientX - rect.left,
        e.clientY - rect.top,
      );
      // Fade the live chip only after the ghost snapshot is captured.
      requestAnimationFrame(() => {
        chip.style.opacity = "0.4";
      });
    }
  }, []);

  const onDragEnd = useCallback((e: DragEvent) => {
    dragRootId.current = null;
    setDraggingRootId(null);
    const chip = (e.target as HTMLElement).closest<HTMLElement>(
      "[data-rootchip]",
    );
    if (chip) chip.style.opacity = "";
  }, []);

  const zoneOver = useCallback((e: DragEvent) => {
    e.preventDefault();
    e.dataTransfer.dropEffect = "move";
  }, []);

  const zoneEnter = useCallback((e: DragEvent) => {
    const el = e.currentTarget as HTMLElement & { _oc?: number };
    el._oc = (el._oc ?? 0) + 1;
    el.style.outline = DROP_OUTLINE;
    el.style.outlineOffset = "2px";
  }, []);

  const zoneLeave = useCallback((e: DragEvent) => {
    const el = e.currentTarget as HTMLElement & { _oc?: number };
    el._oc = (el._oc ?? 0) - 1;
    if (el._oc <= 0) {
      el._oc = 0;
      el.style.outline = "";
      el.style.outlineOffset = "";
    }
  }, []);

  // A root may only drop onto a library whose facet is compatible with the
  // root's source kind (Radarr→movie, Sonarr→series|anime, manual→any). The
  // tray (libraryId === null) always accepts (unassign).
  const dropAllowed = useCallback(
    (libFacet: WizardFacet) => {
      const id = dragRootId.current;
      if (!id) return true;
      const root = rootById(id);
      return !!root && kindCompatibleWithFacet(root.kind, libFacet);
    },
    [rootById],
  );

  const dropTo = useCallback(
    (e: DragEvent, libraryId: string | null) => {
      e.preventDefault();
      const el = e.currentTarget as HTMLElement & { _oc?: number };
      el._oc = 0;
      el.style.outline = "";
      el.style.outlineOffset = "";
      const id = dragRootId.current ?? e.dataTransfer.getData("text/plain");
      dragRootId.current = null;
      setDraggingRootId(null);
      if (!id) return;
      if (libraryId === null) {
        assignRoot(id, null);
        return;
      }
      const root = rootById(id);
      const lib = libraries.find((entry) => entry.id === libraryId);
      if (root && lib && kindCompatibleWithFacet(root.kind, lib.facet)) {
        assignRoot(id, libraryId);
      }
    },
    [assignRoot, libraries, rootById],
  );

  // Facet-aware drag handlers for library cards: refuse the drop (and skip the
  // highlight) when the dragged root's kind is incompatible with the facet.
  const libZoneOver = useCallback(
    (e: DragEvent, libFacet: WizardFacet) => {
      e.preventDefault();
      e.dataTransfer.dropEffect = dropAllowed(libFacet) ? "move" : "none";
    },
    [dropAllowed],
  );
  const libZoneEnter = useCallback(
    (e: DragEvent, libFacet: WizardFacet) => {
      if (!dropAllowed(libFacet)) return;
      zoneEnter(e);
    },
    [dropAllowed, zoneEnter],
  );

  // ── Inline library rename ──────────────────────────────────────────────────
  const startRename = useCallback((libId: string, name: string) => {
    setEditLibId(libId);
    setEditLibVal(name);
  }, []);

  const commitRename = useCallback(() => {
    if (editLibId) {
      const trimmed = editLibVal.trim();
      if (trimmed) renameLibrary(editLibId, trimmed); // keep old name if blank
    }
    setEditLibId(null);
    setEditLibVal("");
  }, [editLibId, editLibVal, renameLibrary]);

  const cancelRename = useCallback(() => {
    setEditLibId(null);
    setEditLibVal("");
  }, []);

  const trayCountLabel =
    trayRoots.length === 1
      ? t("setup.unmappedCountOne", { count: 1 })
      : t("setup.unmappedCountMany", { count: trayRoots.length });
  const draggedRoot = draggingRootId ? rootById(draggingRootId) : null;
  const trayDropState = importLibraryDropState(Boolean(draggedRoot), true);

  const renderChip = (root: ImportRoot, variant: "tray" | "library") => (
    <ImportRootChip
      key={root.id}
      root={root}
      variant={variant}
      invalid={
        Boolean(assign[root.id]) &&
        ((root.manual && !effectiveRootPath(root).trim()) ||
          invalidAssignedRootIdSet.has(root.id))
      }
      draggable={nativeDragEnabled}
      onDragStart={(e) => onDragStart(e, root.id)}
      onDragEnd={onDragEnd}
      onRemap={() => setRemapRootId(root.id)}
      onAssign={() => setAssignSheetRootId(root.id)}
      onRemoveManual={() => removeManualRoot(root.id)}
      onBrowseManual={() => setBrowseTarget({ kind: "manual", rootId: root.id })}
      t={t}
    />
  );

  return (
    <div
      id="setup-import-libraries-view"
      style={{ display: "flex", flexDirection: "column", gap: 0 }}
    >
      {/* ── Source Roots tray ── */}
      <div
        data-drop="tray"
        data-drop-state={trayDropState}
        onDragOver={nativeDragEnabled ? zoneOver : undefined}
        onDragEnter={nativeDragEnabled ? zoneEnter : undefined}
        onDragLeave={nativeDragEnabled ? zoneLeave : undefined}
        onDrop={nativeDragEnabled ? (e) => dropTo(e, null) : undefined}
        style={{
          border: `1px solid ${
            trayDropState === "compatible"
              ? "var(--scry-accent)"
              : "var(--scry-border)"
          }`,
          borderRadius: 16,
          background:
            trayDropState === "compatible"
              ? "rgba(var(--scry-accent-rgb), 0.08)"
              : "rgba(10, 17, 32, 0.5)",
          padding: "18px 20px",
        }}
      >
        <div
          style={{
            display: "flex",
            flexWrap: "wrap",
            alignItems: "center",
            gap: 10,
            marginBottom: 12,
          }}
        >
          <FolderSymlink
            size={17}
            style={{ color: "var(--scry-faint2)", flex: "none" }}
          />
          <span style={SECTION_LABEL}>{t("setup.sourceRoots")}</span>
          <span style={{ fontSize: 12, color: "var(--scry-faint)" }}>
            {trayCountLabel}
          </span>
          <Button
            type="button"
            onClick={() => setBrowseTarget({ kind: "new" })}
            variant="outline"
            size="sm"
            className="ml-auto h-[34px] rounded-[9px] px-3 text-[13px] font-semibold"
          >
            <Plus className="h-4 w-4 text-[var(--scry-accent-text)]" />
            {t("setup.addSourceRoot")}
          </Button>
        </div>

        <div
          role="group"
          aria-label={t("setup.sourceRoots")}
          style={{
            display: "flex",
            flexWrap: "wrap",
            gap: 9,
            minHeight: 46,
            alignItems: "flex-start",
          }}
        >
          {trayRoots.length > 0 ? (
            trayRoots.map((root) => renderChip(root, "tray"))
          ) : previewError ? (
            <div
              style={{
                display: "inline-flex",
                alignItems: "center",
                gap: 10,
                width: "100%",
                minHeight: 46,
                justifyContent: "center",
                borderRadius: 11,
                border: "1px dashed var(--scry-border2)",
                color: "var(--scry-faint)",
                fontSize: 12,
              }}
            >
              <TriangleAlert size={16} style={{ flex: "none" }} />
              {t("setup.previewError")}
              <TextActionButton
                type="button"
                onClick={() => void loadPreview()}
                tone="accent"
                size="sm"
                leadingIcon={<RotateCw size={14} />}
              >
                {t("setup.retry")}
              </TextActionButton>
            </div>
          ) : previewing && roots.length === 0 ? (
            <div
              style={{
                display: "inline-flex",
                alignItems: "center",
                gap: 8,
                width: "100%",
                minHeight: 46,
                justifyContent: "center",
                borderRadius: 11,
                border: "1px dashed var(--scry-border2)",
                color: "var(--scry-faint3)",
                fontSize: 12,
              }}
            >
              <LoadingMark className="size-4" />
              {t("setup.previewLoading")}
            </div>
          ) : roots.length > 0 && trayRoots.length === 0 ? (
            <div
              style={{
                display: "inline-flex",
                alignItems: "center",
                gap: 8,
                width: "100%",
                minHeight: 46,
                justifyContent: "center",
                borderRadius: 11,
                border: "1px dashed var(--scry-border2)",
                color: "var(--scry-faint3)",
                fontSize: 12,
              }}
            >
              <CheckCheck size={16} />
              {t("setup.allRootsMapped")}
            </div>
          ) : null}
        </div>
      </div>

      {/* ── Connector hint ── */}
      <div
        style={{
          display: "flex",
          alignItems: "center",
          justifyContent: "center",
          gap: 7,
          margin: "13px 0",
          fontSize: 12,
          color: "var(--scry-faint3)",
        }}
      >
        <ArrowDown size={15} />
        {t("setup.dragRootIntoLibrary")}
      </div>

      {/* ── Libraries grid ── */}
      <div
        style={{
          display: "grid",
          gridTemplateColumns: "repeat(auto-fill, minmax(266px, 1fr))",
          gap: 14,
        }}
      >
        {libraries.map((lib) => {
          const style = facetStyle(lib.facet);
          const chips = rootsForLibrary(lib.id);
          const editing = editLibId === lib.id;
          const dropState = importLibraryDropState(
            Boolean(draggedRoot),
            !draggedRoot || kindCompatibleWithFacet(draggedRoot.kind, lib.facet),
          );
          return (
            <div
              id={`setup-import-library-drop-${selectorToken(lib.facet)}`}
              key={lib.id}
              data-drop="library"
              data-drop-state={dropState}
              data-library-facet={lib.facet}
              data-library-id={lib.id}
              aria-label={lib.name}
              aria-disabled={dropState === "incompatible"}
              onDragOver={
                nativeDragEnabled ? (e) => libZoneOver(e, lib.facet) : undefined
              }
              onDragEnter={
                nativeDragEnabled ? (e) => libZoneEnter(e, lib.facet) : undefined
              }
              onDragLeave={nativeDragEnabled ? zoneLeave : undefined}
              onDrop={nativeDragEnabled ? (e) => dropTo(e, lib.id) : undefined}
              style={{
                display: "flex",
                flexDirection: "column",
                border: `1px solid ${
                  dropState === "compatible"
                    ? "var(--scry-accent)"
                    : "var(--scry-border)"
                }`,
                borderRadius: 14,
                background:
                  dropState === "compatible"
                    ? "rgba(var(--scry-accent-rgb), 0.08)"
                    : "rgba(10, 17, 32, 0.5)",
                padding: "14px 15px",
                minHeight: 130,
                opacity: dropState === "incompatible" ? 0.45 : 1,
                transition:
                  "border-color 120ms ease, background 120ms ease, opacity 120ms ease",
              }}
            >
              {/* Header */}
              <div
                style={{
                  display: "flex",
                  alignItems: "center",
                  gap: 9,
                  minWidth: 0,
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
                    background: style.bg,
                    border: `1px solid ${style.border}`,
                    color: style.text,
                    flex: "none",
                  }}
                >
                  <Library size={15} />
                </span>
                {editing ? (
                  <input
                    autoFocus
                    autoComplete="off"
                    data-1p-ignore="true"
                    data-lpignore="true"
                    data-bwignore="true"
                    data-form-type="other"
                    data-protonpass-ignore="true"
                    value={editLibVal}
                    onChange={(e) => setEditLibVal(e.target.value)}
                    onFocus={(e) => e.target.select()}
                    onBlur={commitRename}
                    onKeyDown={(e) => {
                      if (e.key === "Enter") {
                        e.preventDefault();
                        commitRename();
                      } else if (e.key === "Escape") {
                        e.preventDefault();
                        cancelRename();
                      }
                    }}
                    style={{
                      width: 130,
                      height: 26,
                      padding: "0 8px",
                      borderRadius: 7,
                      border: "1px solid var(--scry-accent)",
                      background: "var(--scry-bg)",
                      color: "#fff",
                      fontSize: 13.5,
                      fontWeight: 600,
                      outline: "none",
                    }}
                  />
                ) : (
                  <button
                    type="button"
                    title={t("setup.renameLibrary")}
                    aria-label={t("setup.renameLibraryAria", { name: lib.name })}
                    onClick={() => startRename(lib.id, lib.name)}
                    style={{
                      flex: 1,
                      minWidth: 0,
                      textAlign: "left",
                      background: "transparent",
                      border: "none",
                      padding: 0,
                      cursor: "text",
                      fontSize: 14,
                      fontWeight: 600,
                      color: "#f1f5ff",
                      overflow: "hidden",
                      textOverflow: "ellipsis",
                      whiteSpace: "nowrap",
                    }}
                  >
                    {lib.name}
                  </button>
                )}
                <span
                  style={{
                    display: "inline-flex",
                    flex: "none",
                    alignItems: "center",
                    gap: 5,
                    marginLeft: "auto",
                    padding: "3px 9px",
                    borderRadius: 7,
                    fontSize: 10.5,
                    fontWeight: 700,
                    ...facetPillStyle(lib.facet),
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
                {!lib.isDefault ? (
                  <IconButton
                    type="button"
                    label={t("setup.deleteLibrary")}
                    appearance="ghost"
                    tone="delete"
                    onClick={() => removeLibrary(lib.id)}
                    className="h-7 w-7 flex-none rounded-[7px] text-[var(--scry-faint)] hover:text-[var(--scry-danger-text)]"
                  >
                    <Trash2 className="h-3.5 w-3.5" />
                  </IconButton>
                ) : null}
              </div>

              {/* Drop body */}
              <div
                style={{
                  display: "flex",
                  flexDirection: "column",
                  gap: 8,
                  flex: 1,
                }}
              >
                {chips.length > 0 ? (
                  chips.map((root) => renderChip(root, "library"))
                ) : (
                  <div
                    style={{
                      display: "flex",
                      alignItems: "center",
                      justifyContent: "center",
                      gap: 6,
                      minHeight: 44,
                      borderRadius: 10,
                      border: "1px dashed var(--scry-border3)",
                      color: "var(--scry-faint3)",
                      fontSize: 12,
                    }}
                  >
                    <Plus size={13} />
                    {t("setup.dropRootsHere")}
                  </div>
                )}
              </div>
            </div>
          );
        })}

        {/* Add-library card */}
        {addingLib ? (
          <div
            style={{
              display: "flex",
              flexDirection: "column",
              gap: 8,
              minHeight: 130,
              borderRadius: 14,
              border: "1px dashed var(--scry-baccent)",
              background: "rgba(var(--scry-accent-rgb), 0.06)",
              padding: "14px 15px",
            }}
          >
            <span
              style={{
                fontSize: 12.5,
                fontWeight: 600,
                color: "var(--scry-muted2)",
                marginBottom: 2,
              }}
            >
              {t("setup.newLibraryFor")}
            </span>
            {FACET_PICKER.map((facet) => {
              const style = facetStyle(facet);
              return (
                <button
                  key={facet}
                  type="button"
                  onClick={() => {
                    addLibrary(facet);
                    setAddingLib(false);
                  }}
                  style={{
                    display: "inline-flex",
                    alignItems: "center",
                    gap: 7,
                    height: 36,
                    padding: "0 12px",
                    borderRadius: 9,
                    fontSize: 13,
                    fontWeight: 600,
                    cursor: "pointer",
                    ...facetPillStyle(facet),
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
                  {t(facetLabelKey(facet))}
                </button>
              );
            })}
            <Button
              type="button"
              onClick={() => setAddingLib(false)}
              variant="ghost"
              size="sm"
              className="mt-auto h-[30px] px-2 text-[12.5px] font-semibold text-[var(--scry-faint)] hover:bg-transparent hover:text-[var(--scry-muted2)]"
            >
              {t("setup.cancel")}
            </Button>
          </div>
        ) : (
          <AddNewButton
            icon={Plus}
            label={t("setup.addLibrary")}
            onClick={() => setAddingLib(true)}
            className="h-auto min-h-[130px] w-full flex-col gap-3 rounded-[14px] text-[13px]"
          />
        )}
      </div>

      <ImportRootValidationNotice
        checking={assignedRootPathValidationLoading}
        invalidRoots={invalidAssignedRoots}
        onRemap={setRemapRootId}
        t={t}
      />

      {/* ── Remap dialog ── */}
      <ImportRemapDialog
        root={remapRoot}
        open={remapRootId != null}
        onOpenChange={(open) => {
          if (!open) setRemapRootId(null);
        }}
        onSave={(path) => {
          if (remapRootId) setRootRemap(remapRootId, path);
        }}
        t={t}
      />

      {/* ── Mobile assign sheet ── */}
      <ImportAssignSheet
        root={assignSheetRoot}
        libraries={libraries}
        currentLibraryId={
          assignSheetRootId ? assign[assignSheetRootId] ?? null : null
        }
        open={assignSheetRootId != null}
        onOpenChange={(open) => {
          if (!open) setAssignSheetRootId(null);
        }}
        onPick={(libId) => {
          if (assignSheetRootId) assignRoot(assignSheetRootId, libId);
        }}
        onCreateLibrary={(facet) => {
          const id = addLibrary(facet);
          if (assignSheetRootId) assignRoot(assignSheetRootId, id);
          setAssignSheetRootId(null);
        }}
        t={t}
      />

      {/* ── Folder browser for adding / changing manual source roots ── */}
      <FolderBrowserDialog
        open={browseTarget != null}
        onOpenChange={(open) => {
          if (!open) setBrowseTarget(null);
        }}
        onSelect={handleBrowseSelect}
        selectionTypes={["folder"]}
        title={t("setup.addSourceRoot")}
        initialPath={
          browseTarget?.kind === "manual"
            ? rootById(browseTarget.rootId)?.arrRootPath || "/"
            : "/"
        }
      />
    </div>
  );
}
