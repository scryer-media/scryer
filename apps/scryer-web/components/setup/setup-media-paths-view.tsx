import { useState, type KeyboardEvent } from "react";
import { FolderOpen, Plus, X } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { IconButton } from "@/components/ui/icon-button";
import { Input } from "@/components/ui/input";
import {
  SETUP_MEDIA_PATH_FIELDS,
  SETUP_MEDIA_PATH_LABEL_KEYS,
  type SetupMediaPathField,
  type SetupMediaRoots,
} from "@/lib/utils/setup-media-paths";
import { FolderBrowserDialog } from "./folder-browser-dialog";
import {
  SetupBackButton,
  SetupPanel,
  SetupPrimaryButton,
  SetupStepHeader,
} from "./setup-chrome";

interface SetupMediaPathsViewProps {
  t: (key: string) => string;
  roots: SetupMediaRoots;
  onAddRoot: (field: SetupMediaPathField, path: string) => void;
  onReplaceRoot: (field: SetupMediaPathField, index: number, path: string) => void;
  onRemoveRoot: (field: SetupMediaPathField, index: number) => void;
  onSetDefaultRoot: (field: SetupMediaPathField, index: number) => void;
  onNext: () => void;
  onBack: () => void;
  onSkip?: () => void;
  saving: boolean;
  error: string | null;
  invalidPaths?: readonly string[];
  validationUnavailable?: boolean;
}

/** Which row the folder browser fills: an existing root, or a new one. */
type BrowseTarget = { field: SetupMediaPathField; index: number | null } | null;

/** The first row keeps the ids the step had when it held one path per facet. */
function rowId(field: SetupMediaPathField, part: string, index: number) {
  const suffix = index === 0 ? "" : `-${index + 1}`;
  return `setup-media-paths-${field}-${part}${suffix}`;
}

export function SetupMediaPathsView({
  t,
  roots,
  onAddRoot,
  onReplaceRoot,
  onRemoveRoot,
  onSetDefaultRoot,
  onNext,
  onBack,
  onSkip,
  saving,
  error,
  invalidPaths = [],
  validationUnavailable = false,
}: SetupMediaPathsViewProps) {
  const [browseTarget, setBrowseTarget] = useState<BrowseTarget>(null);

  const browseInitialPath = browseTarget
    ? browseTarget.index === null
      ? (roots[browseTarget.field].find((root) => root.isDefault)?.path ?? "/")
      : roots[browseTarget.field][browseTarget.index]?.path
    : "/";

  function handleBrowseSelect(path: string) {
    if (!browseTarget) return;
    if (browseTarget.index === null) onAddRoot(browseTarget.field, path);
    else onReplaceRoot(browseTarget.field, browseTarget.index, path);
  }

  function handlePathInputKeyDown(
    event: KeyboardEvent<HTMLInputElement>,
    target: NonNullable<BrowseTarget>,
  ) {
    if (event.key !== "Enter" && event.key !== " ") {
      return;
    }
    event.preventDefault();
    setBrowseTarget(target);
  }

  return (
    <SetupPanel id="setup-media-paths-view" className="flex flex-col gap-6">
      <SetupStepHeader
        icon={FolderOpen}
        title={t("setup.mediaPathsTitle")}
        subtitle={t("setup.mediaPathsDescription")}
      />
      <div className="mx-auto flex w-full max-w-xl flex-col gap-5">
        {SETUP_MEDIA_PATH_FIELDS.map((field) => {
          const fieldRoots = roots[field];
          const label = t(SETUP_MEDIA_PATH_LABEL_KEYS[field]);
          return (
            <section
              key={field}
              id={`setup-media-paths-${field}`}
              aria-labelledby={`setup-media-paths-${field}-label`}
              className="space-y-2"
            >
              <div className="flex items-center justify-between gap-3">
                <h3
                  id={`setup-media-paths-${field}-label`}
                  className="text-sm font-medium leading-none"
                >
                  {label}
                  <span className="ml-1.5 text-xs font-normal text-muted-foreground">
                    {t("setup.optional")}
                  </span>
                </h3>
                <Button
                  id={`setup-media-paths-${field}-add`}
                  type="button"
                  variant="ghost"
                  size="sm"
                  className="h-8 px-2 text-xs"
                  onClick={() => setBrowseTarget({ field, index: null })}
                >
                  <Plus className="mr-1 h-3.5 w-3.5" />
                  {t("setup.addMediaFolder")}
                </Button>
              </div>
              {fieldRoots.length === 0 ? (
                <p className="text-xs text-muted-foreground">
                  {t("setup.mediaPathsNoneChosen")}
                </p>
              ) : (
                <ul className="space-y-2">
                  {fieldRoots.map((root, index) => {
                    const invalid = invalidPaths.includes(root.path);
                    const target = { field, index };
                    return (
                      <li key={`${root.path}-${index}`} className="space-y-1">
                        <div className="flex items-center gap-2">
                          <div className="min-w-0 flex-1">
                            <Input
                              id={rowId(field, "path", index)}
                              aria-label={label}
                              value={root.path}
                              readOnly
                              onClick={() => setBrowseTarget(target)}
                              onKeyDown={(event) => handlePathInputKeyDown(event, target)}
                              className="cursor-pointer font-[var(--font-code)]"
                              aria-invalid={invalid}
                            />
                          </div>
                          <IconButton
                            id={rowId(field, "browse", index)}
                            label={t("setup.browse")}
                            tone="neutral"
                            onClick={() => setBrowseTarget(target)}
                          >
                            <FolderOpen className="h-4 w-4" />
                          </IconButton>
                          <IconButton
                            id={rowId(field, "clear", index)}
                            label={t("setup.removeMediaFolder")}
                            tone="delete"
                            onClick={() => onRemoveRoot(field, index)}
                          >
                            <X className="h-4 w-4" />
                          </IconButton>
                        </div>
                        {invalid || fieldRoots.length > 1 ? (
                          <div className="flex flex-wrap items-center gap-2 pl-1">
                            {fieldRoots.length > 1 ? (
                              root.isDefault ? (
                                <Badge
                                  tone="info"
                                  className="text-[10px] font-bold uppercase tracking-[0.08em]"
                                >
                                  {t("label.default")}
                                </Badge>
                              ) : (
                                <Button
                                  id={rowId(field, "set-default", index)}
                                  type="button"
                                  variant="link"
                                  size="sm"
                                  className="h-auto p-0 text-xs text-muted-foreground"
                                  onClick={() => onSetDefaultRoot(field, index)}
                                >
                                  {t("settings.rootFolderSetDefault")}
                                </Button>
                              )
                            ) : null}
                            {invalid ? (
                              <Badge
                                tone="warning"
                                className="text-[10px] font-bold uppercase tracking-[0.08em]"
                              >
                                {t("setup.mediaPathNotReachable")}
                              </Badge>
                            ) : null}
                          </div>
                        ) : null}
                      </li>
                    );
                  })}
                </ul>
              )}
            </section>
          );
        })}
        {error && (
          <p id="setup-media-paths-error" data-ui="setup-media-paths-error" className="text-sm text-destructive">
            {error}
          </p>
        )}
        {validationUnavailable && !error ? (
          <p className="text-sm text-[var(--scry-warning-text)]">
            {t("setup.mediaPathsVerificationUnavailable")}
          </p>
        ) : null}
      </div>
      <div className="flex items-center justify-between pt-2">
        <SetupBackButton id="setup-media-paths-back" onClick={onBack}>
          {t("setup.back")}
        </SetupBackButton>
        <div className="flex items-center gap-3">
          {onSkip && (
            <Button id="setup-media-paths-skip" type="button" variant="link" onClick={onSkip}>
              {t("setup.skip")}
            </Button>
          )}
          <SetupPrimaryButton id="setup-media-paths-next" onClick={onNext} disabled={saving}>
            {saving ? t("label.saving") : t("setup.next")}
          </SetupPrimaryButton>
        </div>
      </div>

      <FolderBrowserDialog
        open={browseTarget !== null}
        onOpenChange={(open) => { if (!open) setBrowseTarget(null); }}
        onSelect={handleBrowseSelect}
        selectionTypes={["folder"]}
        initialPath={browseInitialPath || "/"}
        title={t("setup.browse")}
      />
    </SetupPanel>
  );
}
