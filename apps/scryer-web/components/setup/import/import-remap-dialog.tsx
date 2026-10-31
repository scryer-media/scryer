import { useEffect, useState, type CSSProperties } from "react";
import { ArrowDown, ArrowLeftRight, FolderOpen, RotateCcw } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogTitle } from "@/components/ui/dialog";
import { SetupPrimaryButton } from "@/components/setup/setup-chrome";
import { FolderBrowserDialog } from "../folder-browser-dialog";
import {
  effectiveRootPath,
  isRootRemapped,
  type ImportRoot,
} from "@/lib/hooks/use-external-import-setup";

import { ImportInstancePill } from "./import-instance-pill";

interface ImportRemapDialogProps {
  root: ImportRoot | null;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onSave: (scryerPath: string | null) => void;
  t: (key: string, values?: Record<string, unknown>) => string;
}

const MONO: CSSProperties = {
  fontFamily: "var(--font-code)",
};

const LABEL: CSSProperties = {
  display: "block",
  fontSize: 10.5,
  fontWeight: 700,
  letterSpacing: "0.08em",
  textTransform: "uppercase",
  color: "var(--scry-faint3)",
};

/**
 * Remap a detected source root's path to its real location on the Scryer host.
 * The source (provenance) path is read-only; only the Scryer-host path is
 * editable. Saving an empty path or one equal to the source clears the remap.
 */
export function ImportRemapDialog({
  root,
  open,
  onOpenChange,
  onSave,
  t,
}: ImportRemapDialogProps) {
  const [value, setValue] = useState("");
  const [browseOpen, setBrowseOpen] = useState(false);

  // Seed the value from the root's effective path each time the dialog opens.
  useEffect(() => {
    if (open && root) setValue(effectiveRootPath(root));
  }, [open, root]);

  if (!root) return null;

  const remapped = isRootRemapped(root);
  const srcPath = root.arrRootPath || t("setup.noPathSet");

  const handleSave = () => {
    const trimmed = value.trim();
    if (!trimmed || trimmed === root.arrRootPath) onSave(null);
    else onSave(trimmed);
    onOpenChange(false);
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent
        showCloseButton
        className="gap-0 p-[22px] sm:max-w-[448px]"
        style={{
          background: "var(--scry-page2)",
          border: "1px solid var(--scry-border)",
          borderRadius: 16,
        }}
      >
        {/* Header */}
        <div style={{ display: "flex", alignItems: "center", gap: 11 }}>
          <span
            aria-hidden
            style={{
              display: "inline-flex",
              alignItems: "center",
              justifyContent: "center",
              width: 34,
              height: 34,
              borderRadius: 9,
              background: "rgba(var(--scry-accent-rgb), 0.12)",
              border: "1px solid var(--scry-baccent)",
              color: "var(--scry-accent-text)",
              flex: "none",
            }}
          >
            <ArrowLeftRight size={17} />
          </span>
          <DialogTitle
            style={{
              fontFamily: "var(--font-space-grotesk)",
              fontSize: 17,
              fontWeight: 700,
              color: "#fff",
            }}
          >
            {t("setup.remapTitle")}
          </DialogTitle>
        </div>

        {/* Explanation */}
        <p
          style={{
            marginTop: 14,
            fontSize: 13,
            lineHeight: 1.55,
            color: "var(--scry-muted)",
          }}
        >
          {t("setup.remapExplain", { instance: root.instanceLabel })}
        </p>

        {/* As reported by source (read-only) */}
        <span style={{ ...LABEL, marginTop: 18 }}>
          {t("setup.remapAsReported")}
        </span>
        <div
          style={{
            display: "flex",
            alignItems: "center",
            gap: 9,
            marginTop: 8,
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
            title={srcPath}
            style={{
              ...MONO,
              fontSize: 13,
              color: "var(--scry-ink2)",
              overflow: "hidden",
              textOverflow: "ellipsis",
              whiteSpace: "nowrap",
            }}
          >
            {srcPath}
          </span>
        </div>

        {/* Down arrow */}
        <div
          aria-hidden
          style={{
            display: "flex",
            justifyContent: "center",
            margin: "10px 0",
            color: "var(--scry-accent-text)",
          }}
        >
          <ArrowDown size={16} />
        </div>

        {/* Path on Scryer host — chosen via the folder browser, never typed. */}
        <span style={LABEL}>{t("setup.remapScryerHostPath")}</span>
        <button
          type="button"
          onClick={() => setBrowseOpen(true)}
          title={value || t("setup.chooseFolder")}
          style={{
            ...MONO,
            display: "flex",
            alignItems: "center",
            gap: 8,
            marginTop: 8,
            width: "100%",
            height: 42,
            padding: "0 12px",
            borderRadius: 10,
            border: "1px solid var(--scry-baccent)",
            background: "var(--scry-bg)",
            color: value ? "#fff" : "var(--scry-faint)",
            fontSize: 14,
            cursor: "pointer",
            textAlign: "left",
          }}
        >
          <FolderOpen
            size={15}
            style={{ flex: "none", color: "var(--scry-accent-text)" }}
          />
          <span
            style={{
              flex: 1,
              overflow: "hidden",
              textOverflow: "ellipsis",
              whiteSpace: "nowrap",
            }}
          >
            {value || t("setup.chooseFolder")}
          </span>
        </button>

        {/* Footer */}
        <div
          style={{
            display: "flex",
            alignItems: "center",
            gap: 8,
            marginTop: 20,
          }}
        >
          {remapped ? (
            <Button
              variant="ghost"
              size="sm"
              onClick={() => {
                onSave(null);
                onOpenChange(false);
              }}
            >
              <RotateCcw />
              {t("setup.remapResetToSource")}
            </Button>
          ) : null}
          <span style={{ flex: 1 }} />
          <Button
            variant="outline"
            size="sm"
            onClick={() => onOpenChange(false)}
          >
            {t("setup.cancel")}
          </Button>
          <SetupPrimaryButton
            size="sm"
            className="shadow-none"
            onClick={handleSave}
          >
            {t("setup.remapSave")}
          </SetupPrimaryButton>
        </div>

        <FolderBrowserDialog
          open={browseOpen}
          onOpenChange={setBrowseOpen}
          onSelect={(path) => setValue(path)}
          selectionTypes={["folder"]}
          title={t("setup.remapScryerHostPath")}
          initialPath={value || root.arrRootPath || "/"}
        />
      </DialogContent>
    </Dialog>
  );
}
