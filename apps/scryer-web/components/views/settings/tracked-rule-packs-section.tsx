import * as React from "react";
import { Copy, RefreshCw, Trash2 } from "lucide-react";
import { ConfirmDialog } from "@/components/common/confirm-dialog";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { IconButton } from "@/components/ui/icon-button";
import { Input, signedIntegerInputProps } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { selectorId } from "@/lib/utils/dom-ids";

export type TrackedRulePackMember = {
  templateId: string;
  ruleSetId: string;
  removed: boolean;
  enabled: boolean | null;
  priority: number | null;
  name: string | null;
  description: string | null;
  appliedFacets: string[];
};

export type TrackedRulePackRecord = {
  packId: string;
  name: string;
  version: string;
  digest: string;
  revision: number;
  availableVersion: string | null;
  autoUpdate: boolean;
  autoUpdateAvailable: boolean;
  lastError: string | null;
  lastUpdated: string | null;
  members: TrackedRulePackMember[];
};

export type TrackedRulePackPreview = {
  version: string;
  digest: string;
  revision: number;
  addedTemplateIds: string[];
  changedTemplateIds: string[];
  removedTemplateIds: string[];
};

type TrackedRulePacksSectionProps = {
  packs: TrackedRulePackRecord[];
  canManage: boolean;
  mutatingPackId: string | null;
  mutatingRuleSetId: string | null;
  onPreviewUpdate: (pack: TrackedRulePackRecord) => Promise<TrackedRulePackPreview | null>;
  onApplyUpdate: (pack: TrackedRulePackRecord, preview: TrackedRulePackPreview) => Promise<void>;
  onSetAutoUpdate: (pack: TrackedRulePackRecord, enabled: boolean) => Promise<boolean>;
  onUninstall: (pack: TrackedRulePackRecord) => Promise<void>;
  onToggleMember: (pack: TrackedRulePackRecord, member: TrackedRulePackMember) => Promise<boolean>;
  onUpdateMemberPriority: (pack: TrackedRulePackRecord, member: TrackedRulePackMember, priority: number) => Promise<boolean>;
  onCopyMember: (member: TrackedRulePackMember) => void;
};

function FacetBadges({ facets }: { facets: string[] }) {
  return facets.length === 0 ? (
    <Badge tone="info" className="capitalize">Global</Badge>
  ) : (
    <div className="flex flex-wrap gap-1">
      {facets.map((facet) => (
        <Badge key={facet} tone="neutral" className="capitalize">{facet}</Badge>
      ))}
    </div>
  );
}

function ChangeSummary({ preview }: { preview: TrackedRulePackPreview }) {
  const groups = [
    ["Added", preview.addedTemplateIds],
    ["Changed", preview.changedTemplateIds],
    ["Removed", preview.removedTemplateIds],
  ] as const;
  return (
    <div className="space-y-2 text-xs text-muted-foreground">
      <p>Version {preview.version} is ready to apply.</p>
      {groups.map(([label, templateIds]) => (
        <div key={label}>
          <span className="font-medium text-foreground">{label} ({templateIds.length})</span>
          {templateIds.length > 0 ? <p className="mt-1 break-words">{templateIds.join(", ")}</p> : null}
        </div>
      ))}
    </div>
  );
}

const MIN_I32 = -(2 ** 31);
const MAX_I32 = 2 ** 31 - 1;

export function isI32(value: number): boolean {
  return Number.isInteger(value) && value >= MIN_I32 && value <= MAX_I32;
}

export function TrackedRulePacksSection({
  packs,
  canManage,
  mutatingPackId,
  mutatingRuleSetId,
  onPreviewUpdate,
  onApplyUpdate,
  onSetAutoUpdate,
  onUninstall,
  onToggleMember,
  onUpdateMemberPriority,
  onCopyMember,
}: TrackedRulePacksSectionProps) {
  const [preview, setPreview] = React.useState<{
    pack: TrackedRulePackRecord;
    changes: TrackedRulePackPreview;
  } | null>(null);
  const [pendingUninstall, setPendingUninstall] = React.useState<TrackedRulePackRecord | null>(null);
  const [priorityDrafts, setPriorityDrafts] = React.useState<Record<string, string>>({});

  React.useEffect(() => {
    setPriorityDrafts({});
  }, [packs]);

  if (packs.length === 0) return null;

  return (
    <section id="settings-tracked-rule-packs" className="space-y-3">
      <div>
        <h3 className="text-base font-semibold">Installed rule packs</h3>
        <p className="text-xs text-muted-foreground">
          Pack-authored names, descriptions, source, and facets stay managed. Copy an individual rule to customize it.
        </p>
      </div>
      {packs.map((pack) => {
        const busy = mutatingPackId === pack.packId;
        return (
          <div key={pack.packId} className="overflow-hidden rounded border border-border bg-card">
            <div className="flex flex-wrap items-center gap-3 border-b border-border px-3 py-2">
              <div className="min-w-0 flex-1">
                <p className="font-medium">{pack.name}</p>
                <p className="text-xs text-muted-foreground">{pack.packId} · installed {pack.version}</p>
              </div>
              {pack.availableVersion ? (
                <Badge tone="info">Update {pack.availableVersion} available</Badge>
              ) : null}
              {canManage ? <label className="flex items-center gap-2 text-xs">
                <Checkbox
                  id={selectorId("settings-tracked-rule-pack-auto-update", pack.packId)}
                  checked={pack.autoUpdate}
                  disabled={busy}
                  onCheckedChange={(value) => void onSetAutoUpdate(pack, value === true)}
                />
                <Label htmlFor={selectorId("settings-tracked-rule-pack-auto-update", pack.packId)}>Auto-update</Label>
              </label> : null}
              <p className="basis-full text-xs text-muted-foreground">
                Installs compatible stable patches for this pack’s current major and minor version, independently of plugin auto-update settings.
              </p>
              {canManage ? <Button
                id={selectorId("settings-tracked-rule-pack-update", pack.packId)}
                type="button"
                variant="secondary"
                disabled={busy}
                onClick={() => void onPreviewUpdate(pack).then((changes) => {
                  if (changes) setPreview({ pack, changes });
                })}
              >
                <RefreshCw className="mr-2 h-4 w-4" />Check for updates
              </Button> : null}
              {canManage ? <IconButton
                id={selectorId("settings-tracked-rule-pack-uninstall", pack.packId)}
                label={`Uninstall ${pack.name}`}
                tone="delete"
                disabled={busy}
                onClick={() => setPendingUninstall(pack)}
              >
                <Trash2 className="h-4 w-4" />
              </IconButton> : null}
            </div>
            {pack.lastError ? <p className="border-b border-[var(--scry-danger-border)] bg-[var(--scry-danger-bg)] px-3 py-2 text-xs text-[var(--scry-danger-text)]">{pack.lastError}</p> : null}
            <div className="overflow-x-auto">
              <Table>
                <TableHeader><TableRow>
                  <TableHead>Name</TableHead><TableHead>Description</TableHead><TableHead>Facets</TableHead>
                  <TableHead className="w-28 text-center">Priority</TableHead><TableHead className="w-24 text-center">Enabled</TableHead><TableHead className="w-16 text-right">Actions</TableHead>
                </TableRow></TableHeader>
                <TableBody>
                  {pack.members.map((member) => {
                    const memberBusy = busy || mutatingRuleSetId === member.ruleSetId;
                    const missingRule = member.enabled === null || member.priority === null || member.name === null;
                    const priorityKey = `${pack.packId}:${member.templateId}`;
                    const priorityValue = priorityDrafts[priorityKey] ?? String(member.priority ?? 0);
                    return <TableRow key={member.templateId} data-ui="settings-table-row">
                      <TableCell className="font-medium">{member.name ?? member.templateId}{member.removed ? <Badge tone="neutral" className="ml-2">Removed upstream</Badge> : null}</TableCell>
                      <TableCell className="max-w-[240px] truncate text-muted-foreground">{member.description || "—"}</TableCell>
                      <TableCell><FacetBadges facets={member.appliedFacets} /></TableCell>
                      <TableCell><Input {...signedIntegerInputProps} aria-label={`Priority: ${member.name ?? member.templateId}`} value={priorityValue} disabled={!canManage || member.removed || missingRule || memberBusy} onChange={(event) => setPriorityDrafts((current) => ({ ...current, [priorityKey]: event.target.value }))} onBlur={(event) => {
                        const priority = Number(event.currentTarget.value);
                        if (!isI32(priority)) {
                          setPriorityDrafts((current) => ({ ...current, [priorityKey]: String(member.priority ?? 0) }));
                          return;
                        }
                        if (priority !== member.priority) {
                          void onUpdateMemberPriority(pack, member, priority).then((updated) => {
                            if (!updated) setPriorityDrafts((current) => ({ ...current, [priorityKey]: String(member.priority ?? 0) }));
                          });
                        }
                      }} /></TableCell>
                      <TableCell className="text-center"><Checkbox aria-label={`Enabled: ${member.name ?? member.templateId}`} checked={member.enabled ?? false} disabled={!canManage || member.removed || missingRule || memberBusy} onCheckedChange={() => void onToggleMember(pack, member)} /></TableCell>
                      <TableCell className="text-right">{canManage ? <IconButton id={selectorId("settings-tracked-rule-pack-copy", member.templateId)} label={`Copy ${member.name ?? member.templateId} as custom`} tone="neutral" disabled={missingRule || memberBusy} onClick={() => onCopyMember(member)}><Copy className="h-4 w-4" /></IconButton> : null}</TableCell>
                    </TableRow>;
                  })}
                </TableBody>
              </Table>
            </div>
          </div>
        );
      })}
      <ConfirmDialog
        open={canManage && preview !== null}
        title="Apply rule pack update"
        description={preview ? `Review changes for ${preview.pack.name} before applying.` : ""}
        confirmLabel="Apply update"
        cancelLabel="Cancel"
        isBusy={preview !== null && mutatingPackId === preview.pack.packId}
        onCancel={() => setPreview(null)}
        onConfirm={async () => {
          if (!preview) return;
          await onApplyUpdate(preview.pack, preview.changes);
          setPreview(null);
        }}
      >
        {preview ? <ChangeSummary preview={preview.changes} /> : null}
      </ConfirmDialog>
      <ConfirmDialog
        open={canManage && pendingUninstall !== null}
        title="Uninstall rule pack"
        description={pendingUninstall ? `Remove ${pendingUninstall.name} and its tracked rules? Custom copies remain.` : ""}
        confirmLabel="Uninstall"
        cancelLabel="Cancel"
        isBusy={pendingUninstall !== null && mutatingPackId === pendingUninstall.packId}
        onCancel={() => setPendingUninstall(null)}
        onConfirm={async () => {
          if (!pendingUninstall) return;
          await onUninstall(pendingUninstall);
          setPendingUninstall(null);
        }}
      />
    </section>
  );
}
