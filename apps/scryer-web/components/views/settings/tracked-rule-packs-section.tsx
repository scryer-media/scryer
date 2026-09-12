import * as React from "react";
import { ChevronRight, Copy, RefreshCw, Trash2 } from "lucide-react";
import { ConfirmDialog } from "@/components/common/confirm-dialog";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { IconButton } from "@/components/ui/icon-button";
import { Label } from "@/components/ui/label";
import { RuleSetTestPanel } from "@/components/containers/settings/rule-set-test-panel";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { useUiDateTimeFormat } from "@/lib/context/ui-settings-context";
import { formatUiDateTime } from "@/lib/utils/date-format";
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
  customizable?: boolean | null;
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
  onCopyMember: (member: TrackedRulePackMember) => void;
  defaultExpandedPackIds?: string[];
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
  onCopyMember,
  defaultExpandedPackIds,
}: TrackedRulePacksSectionProps) {
  const dateTimeFormat = useUiDateTimeFormat();
  const [preview, setPreview] = React.useState<{
    pack: TrackedRulePackRecord;
    changes: TrackedRulePackPreview;
  } | null>(null);
  const [pendingUninstall, setPendingUninstall] = React.useState<TrackedRulePackRecord | null>(null);
  const [expandedPackIds, setExpandedPackIds] = React.useState<Set<string>>(() => new Set(defaultExpandedPackIds ?? []));
  const [testingMember, setTestingMember] = React.useState<{
    packId: string;
    templateId: string;
    ruleSetId: string;
  } | null>(null);

  React.useEffect(() => {
    setTestingMember(null);
  }, [packs]);

  if (packs.length === 0) return null;

  return (
    <section id="settings-tracked-rule-packs" className="space-y-3">
      <div>
        <h3 className="text-base font-semibold">Installed rule packs</h3>
        <p className="text-xs text-muted-foreground">
          Pack-authored names, descriptions, and facets stay managed. Some packs allow individual rules to be copied for customization.
        </p>
      </div>
      <div className="overflow-x-auto rounded border border-border bg-card">
        <Table layout="fixed" className="min-w-[1080px]" aria-label="Installed rule packs">
          <TableHeader><TableRow>
            <TableHead>Pack</TableHead>
            <TableHead className="w-40">Version</TableHead>
            <TableHead className="w-20 text-center">Rules</TableHead>
            <TableHead className="w-52">Last updated</TableHead>
            <TableHead className="w-36 text-center">Auto-update</TableHead>
            <TableHead className="w-40 text-right">Actions</TableHead>
          </TableRow></TableHeader>
          <TableBody>
            {packs.map((pack) => {
              const busy = mutatingPackId === pack.packId;
              const customizable = pack.customizable !== false;
              const expanded = expandedPackIds.has(pack.packId);
              const detailId = selectorId("settings-tracked-rule-pack-details", pack.packId);
              const autoUpdateId = selectorId("settings-tracked-rule-pack-auto-update", pack.packId);
              return <React.Fragment key={pack.packId}>
                <TableRow data-ui="settings-table-row" className="hover:bg-[var(--scry-rowHover)]">
                  <TableCell className="py-0">
                    <button
                      id={selectorId("settings-tracked-rule-pack-disclosure", pack.packId)}
                      type="button"
                      className="flex w-full items-center gap-2 py-3 text-left"
                      aria-expanded={expanded}
                      aria-controls={detailId}
                      onClick={() => setExpandedPackIds((current) => {
                        const next = new Set(current);
                        if (next.has(pack.packId)) next.delete(pack.packId);
                        else next.add(pack.packId);
                        return next;
                      })}
                    >
                      <ChevronRight className={`h-4 w-4 shrink-0 text-muted-foreground transition-transform ${expanded ? "rotate-90" : ""}`} />
                      <span className="min-w-0"><span className="block truncate font-medium">{pack.name}</span><span className="block truncate text-xs text-muted-foreground">{pack.packId}</span>{!customizable ? <span className="block truncate text-xs text-muted-foreground">Managed by the pack author.</span> : null}</span>
                    </button>
                  </TableCell>
                  <TableCell className="text-sm text-muted-foreground">
                    <span className="block text-foreground">{pack.version}</span>
                    {pack.availableVersion ? <Badge tone="info" className="mt-1">Update {pack.availableVersion} available</Badge> : null}
                    {pack.lastError ? <span className="block text-xs text-[var(--scry-danger-text)]">Update failed</span> : null}
                  </TableCell>
                  <TableCell className="text-center tabular-nums text-muted-foreground">{pack.members.length}</TableCell>
                  <TableCell className="text-sm text-muted-foreground">
                    {pack.lastUpdated ? <time dateTime={pack.lastUpdated}>{formatUiDateTime(pack.lastUpdated, dateTimeFormat, { fallback: "—" })}</time> : "—"}
                  </TableCell>
                  <TableCell className="text-center">{canManage ? <div className="inline-flex items-center">
                    <Checkbox id={autoUpdateId} checked={pack.autoUpdate} disabled={busy} onCheckedChange={(value) => void onSetAutoUpdate(pack, value === true)} />
                    <Label htmlFor={autoUpdateId} className="sr-only">Auto-update {pack.name}</Label>
                  </div> : <span className="text-xs text-muted-foreground">{pack.autoUpdate ? "On" : "Off"}</span>}</TableCell>
                  <TableCell className="text-right">{canManage ? <div className="flex justify-end gap-2"><Button id={selectorId("settings-tracked-rule-pack-update", pack.packId)} type="button" variant="secondary" size="sm" disabled={busy} onClick={() => void onPreviewUpdate(pack).then((changes) => { if (changes) setPreview({ pack, changes }); })}><RefreshCw className="mr-2 h-4 w-4" />Check</Button><IconButton id={selectorId("settings-tracked-rule-pack-uninstall", pack.packId)} label={`Uninstall ${pack.name}`} tone="delete" disabled={busy} onClick={() => setPendingUninstall(pack)}><Trash2 className="h-4 w-4" /></IconButton></div> : null}</TableCell>
                </TableRow>
                {expanded ? <TableRow id={detailId} data-ui="settings-table-row">
                  <TableCell colSpan={6} className="p-4">
                    {pack.lastError ? <p className="mb-3 text-xs text-[var(--scry-danger-text)]">{pack.lastError}</p> : null}
                    <Table layout="fixed" density="dense" aria-label={`${pack.name} rules`} wrapperClassName="rounded border border-border bg-card">
                      <TableHeader><TableRow>
                        <TableHead className="w-[28%]">Rule</TableHead>
                        <TableHead>Description</TableHead>
                        <TableHead className="w-32">Facets</TableHead>
                        <TableHead className="w-20 text-center">Enabled</TableHead>
                        <TableHead className="w-32 text-right">Actions</TableHead>
                      </TableRow></TableHeader>
                      <TableBody>
                  {pack.members.map((member) => {
                    const memberBusy = busy || mutatingRuleSetId === member.ruleSetId;
                    const missingRule = member.enabled === null || member.priority === null || member.name === null;
                    const testing = testingMember?.packId === pack.packId && testingMember.templateId === member.templateId;
                    return <React.Fragment key={member.templateId}>
                      <TableRow data-ui="settings-table-row">
                      <TableCell className="font-medium">{member.name ?? member.templateId}{member.removed ? <Badge tone="neutral" className="ml-2">Removed upstream</Badge> : null}</TableCell>
                      <TableCell className="text-muted-foreground">{member.description || "—"}</TableCell>
                      <TableCell><FacetBadges facets={member.appliedFacets} /></TableCell>
                      <TableCell className="text-center"><Checkbox aria-label={`Enabled: ${member.name ?? member.templateId}`} checked={member.enabled ?? false} disabled={!canManage || member.removed || missingRule || memberBusy} onCheckedChange={() => void onToggleMember(pack, member)} /></TableCell>
                      <TableCell className="text-right"><div className="flex justify-end gap-1">
                        <Button id={selectorId("settings-tracked-rule-pack-test", member.templateId)} type="button" variant="secondary" size="sm" disabled={member.removed || missingRule || !member.ruleSetId || memberBusy} onClick={() => setTestingMember(testing ? null : { packId: pack.packId, templateId: member.templateId, ruleSetId: member.ruleSetId })}>Test</Button>
                        {canManage && customizable ? <IconButton id={selectorId("settings-tracked-rule-pack-copy", member.templateId)} label={`Copy ${member.name ?? member.templateId} as custom`} tone="neutral" disabled={missingRule || memberBusy} onClick={() => onCopyMember(member)}><Copy className="h-4 w-4" /></IconButton> : null}
                      </div></TableCell>
                      </TableRow>
                      {testing ? <TableRow data-ui="settings-table-row"><TableCell colSpan={5} className="bg-muted/20 p-3"><RuleSetTestPanel key={member.ruleSetId} draft={null} editRuleSetId={null} copySourceRuleSetId={null} testRuleSetId={member.ruleSetId} open onOpenChange={(nextOpen) => { if (!nextOpen) setTestingMember(null); }} /></TableCell></TableRow> : null}
                    </React.Fragment>;
                  })}
                      </TableBody>
                    </Table>
                  </TableCell>
                </TableRow> : null}
              </React.Fragment>;
            })}
          </TableBody>
        </Table>
      </div>
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
