import * as React from "react";
import {
  BookOpen,
  ChevronDown,
  Copy,
  Edit,
  Library,
  Plus,
  Power,
  Trash2,
} from "lucide-react";
import { useClient } from "urql";
import { AddNewButton } from "@/components/common/add-new-button";
import {
  RULE_TEMPLATES,
  RULE_TEMPLATE_CATEGORIES,
  type RuleTemplate,
} from "@/lib/constants/rule-templates";
import {
  rulePackRegistryQuery,
  rulePackTemplatesQuery,
} from "@/lib/graphql/queries";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from "@/components/ui/collapsible";
import { IconButton } from "@/components/ui/icon-button";
import { cn } from "@/lib/utils";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Input,
  integerInputProps,
  sanitizeDigits,
} from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  LazyRegoEditor,
  type RegoEditorDiagnostic,
} from "@/components/common/lazy-rego-editor";
import { RenderBooleanIcon } from "@/components/common/boolean-icon";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { useTranslate } from "@/lib/context/translate-context";
import type {
  RuleSetRecord,
  RuleSetDraft,
  RuleValidationResult,
} from "@/lib/types/rule-sets";
import ruleInputContract from "@/lib/contracts/rule-input-contract.json";
import { selectorId } from "@/lib/utils/dom-ids";
import { isUserOwnedRuleSet } from "@/lib/utils/rule-sets";
import { trashLocalePacks } from "@/lib/utils/trash-packs";

type SettingsRulesSectionProps = {
  isEditorOpen: boolean;
  editorMode: "create" | "edit";
  editingRuleSetId: string | null;
  ruleSetDraft: RuleSetDraft;
  setRuleSetDraft: React.Dispatch<React.SetStateAction<RuleSetDraft>>;
  submitRuleSet: (
    event: React.FormEvent<HTMLFormElement>,
  ) => Promise<void> | void;
  mutatingRuleSetId: string | null;
  resetRuleSetDraft: () => void;
  startCreateRuleSet: () => void;
  ruleSetRecords: RuleSetRecord[];
  copyRuleSet: (record: RuleSetRecord) => void;
  editRuleSet: (record: RuleSetRecord) => void;
  toggleRuleSetEnabled: (record: RuleSetRecord) => Promise<void> | void;
  saveManagedTagFilter: (
    record: RuleSetRecord,
    raw: string,
  ) => Promise<void> | void;
  deleteRuleSet: (record: RuleSetRecord) => Promise<void> | void;
  validateDraft: () => Promise<void> | void;
  validating: boolean;
  validationResult: RuleValidationResult | null;
  applyTemplate: (template: {
    title: string;
    description: string;
    regoSource: string;
    appliedFacets?: string[];
  }) => void;
};

const FACET_OPTIONS = [
  { value: "movie", label: "Movie" },
  { value: "series", label: "Series" },
  { value: "anime", label: "Anime" },
];

type RefField = { field: string; type: string; descKey: string };

type RefSectionDef = { titleKey: string; path: string; fields: RefField[] };
const REF_SECTIONS = ruleInputContract.sections as RefSectionDef[];
// The validator prepends one hidden import line before compiling user-authored Rego.
const RULE_VALIDATION_HIDDEN_LINE_OFFSET = 1;

function toVisibleRuleLine(line: number): number {
  return Math.max(1, line - RULE_VALIDATION_HIDDEN_LINE_OFFSET);
}

function adjustRuleValidationLocations(text: string): string {
  return text.replace(/(\S+\.rego:)(\d+)(:\d+)/g, (_, prefix: string, lineText: string, suffix: string) => {
    const line = Number.parseInt(lineText, 10);
    return Number.isFinite(line)
      ? `${prefix}${toVisibleRuleLine(line)}${suffix}`
      : `${prefix}${lineText}${suffix}`;
  });
}

function formatRuleValidationError(error: string): string {
  const normalized = error.replace(/\r\n/g, "\n").trim();
  if (normalized.includes("\n")) {
    return adjustRuleValidationLocations(normalized).replace(
      /^(\s*)(\d+)(\s+\|)/gm,
      (match, prefix: string, lineText: string, suffix: string) => {
        const line = Number.parseInt(lineText, 10);
        return Number.isFinite(line)
          ? `${prefix}${toVisibleRuleLine(line)}${suffix}`
          : match;
      },
    );
  }

  const parts = normalized.split(/\s+\|\s+/);
  if (parts.length < 4) {
    return normalized;
  }

  const [location, lineNumber, source, ...hintParts] = parts;
  const rawLine = Number.parseInt(lineNumber, 10);
  const visibleLineNumber = Number.isFinite(rawLine)
    ? String(toVisibleRuleLine(rawLine))
    : lineNumber;
  const gutterWidth = Math.max(visibleLineNumber.length, 1);
  const columnMatch = location.match(/:(\d+):(\d+)(?:\D*$|$)/);
  const column = columnMatch ? Number.parseInt(columnMatch[2] ?? "", 10) : null;
  const hint = hintParts.join(" | ").trim();
  const visibleLocation = adjustRuleValidationLocations(location.trim());
  const locationMatch = visibleLocation.match(/^(.*?:)\s*(-->\s+.+)$/);
  const locationLines = locationMatch
    ? [locationMatch[1], locationMatch[2]]
    : [visibleLocation];
  const pointerErrorMatch = hint.match(/^(\^+)\s+((?:error|warning|note):\s*.+)$/);
  const pointerMarker = pointerErrorMatch?.[1] ?? hint;
  const trailingDiagnostic = pointerErrorMatch?.[2];
  const pointer =
    column && Number.isFinite(column) && pointerMarker.startsWith("^")
      ? `${" ".repeat(Math.max(0, column - 1))}${pointerMarker}`
      : pointerMarker;

  return [
    ...locationLines,
    `${" ".repeat(gutterWidth)} |`,
    `${visibleLineNumber.padStart(gutterWidth)} | ${source.trim()}`,
    `${" ".repeat(gutterWidth)} | ${pointer}`,
    trailingDiagnostic,
  ].filter((line): line is string => Boolean(line)).join("\n");
}

function parseRuleValidationDiagnostic(error: string): RegoEditorDiagnostic | null {
  const match = error.match(/\S+\.rego:(\d+):(\d+)/);
  if (!match) {
    return null;
  }

  const rawLine = Number.parseInt(match[1] ?? "", 10);
  const column = Number.parseInt(match[2] ?? "", 10);
  if (!Number.isFinite(rawLine) || rawLine < 1) {
    return null;
  }

  return {
    line: toVisibleRuleLine(rawLine),
    column: Number.isFinite(column) && column > 0 ? column : null,
    message: formatRuleValidationError(error),
  };
}

function getRuleValidationDiagnostics(
  validationResult: RuleValidationResult | null,
): RegoEditorDiagnostic[] {
  if (!validationResult || validationResult.valid) {
    return [];
  }

  return validationResult.errors
    .map(parseRuleValidationDiagnostic)
    .filter((diagnostic): diagnostic is RegoEditorDiagnostic => Boolean(diagnostic));
}

function RefFieldTable({ section }: { section: RefSectionDef }) {
  const t = useTranslate();
  return (
    <div>
      <h4 className="mb-1 font-semibold">
        <code className="rounded bg-muted px-1.5 py-0.5 text-xs">
          {section.path}
        </code>{" "}
        <span className="text-muted-foreground font-normal">
          {t(section.titleKey)}
        </span>
      </h4>
      <div className="overflow-x-auto">
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead className="w-[220px]">
                {t("settings.refColField")}
              </TableHead>
              <TableHead className="w-[100px]">
                {t("settings.refColType")}
              </TableHead>
              <TableHead>{t("settings.refColDescription")}</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {section.fields.map((f) => (
              <TableRow key={f.field}>
                <TableCell>
                  <code className="text-xs">
                    {section.path}.{f.field}
                  </code>
                </TableCell>
                <TableCell>
                  <code className="rounded bg-muted px-1 py-0.5 text-xs text-muted-foreground">
                    {f.type}
                  </code>
                </TableCell>
                <TableCell className="text-muted-foreground">
                  {t(f.descKey)}
                </TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      </div>
    </div>
  );
}

function RulesContextReference() {
  const t = useTranslate();
  const [open, setOpen] = React.useState(false);

  return (
    <Card>
      <CardHeader
        className="cursor-pointer select-none"
        onClick={() => setOpen((prev) => !prev)}
      >
        <CardTitle className="flex items-center gap-2 text-base">
          <BookOpen className="h-4 w-4" />
          {t("settings.refTitle")}
          <ChevronDown
            className={`ml-auto h-4 w-4 transition-transform ${open ? "rotate-180" : ""}`}
          />
        </CardTitle>
        <p className="text-xs text-muted-foreground">
          {t("settings.refSubtitle")}
        </p>
      </CardHeader>
      {open ? (
        <CardContent className="space-y-6 text-sm">
          <p className="text-muted-foreground">{t("settings.refIntro")}</p>

          <div>
            <h4 className="mb-2 font-semibold">
              {t("settings.refSectionSandbox")}
            </h4>
            <p className="mb-2 text-muted-foreground">
              {t("settings.refSandboxIntro")}
            </p>
            <ul className="list-disc space-y-1.5 pl-5 text-muted-foreground">
              <li>{t("settings.refSandboxNoIO")}</li>
              <li>{t("settings.refSandboxPkgIsolation")}</li>
              <li>{t("settings.refSandboxReadOnly")}</li>
              <li>{t("settings.refSandboxOutputRestricted")}</li>
              <li>{t("settings.refSandboxIntegerOnly")}</li>
              <li>{t("settings.refSandboxValidation")}</li>
              <li>{t("settings.refSandboxErrorIsolation")}</li>
            </ul>
          </div>

          {REF_SECTIONS.map((section) => (
            <RefFieldTable key={section.path} section={section} />
          ))}

          <div>
            <h4 className="mb-1 font-semibold">
              {t("settings.refSectionBuiltins")}
            </h4>
            <p className="mb-2 text-muted-foreground">
              {t("settings.refBuiltinsIntro")}
            </p>
            <div className="overflow-x-auto">
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead className="w-[280px]">
                      {t("settings.refColFunction")}
                    </TableHead>
                    <TableHead className="w-[100px]">
                      {t("settings.refColReturns")}
                    </TableHead>
                    <TableHead>{t("settings.refColDescription")}</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  <TableRow>
                    <TableCell>
                      <code className="text-xs">scryer.block_score()</code>
                    </TableCell>
                    <TableCell>
                      <code className="rounded bg-muted px-1 py-0.5 text-xs text-muted-foreground">
                        number
                      </code>
                    </TableCell>
                    <TableCell className="text-muted-foreground">
                      {t("settings.refFnBlockScore")}
                    </TableCell>
                  </TableRow>
                  <TableRow>
                    <TableCell>
                      <code className="text-xs">scryer.size_gib(bytes)</code>
                    </TableCell>
                    <TableCell>
                      <code className="rounded bg-muted px-1 py-0.5 text-xs text-muted-foreground">
                        float
                      </code>
                    </TableCell>
                    <TableCell className="text-muted-foreground">
                      {t("settings.refFnSizeGib")}
                    </TableCell>
                  </TableRow>
                  <TableRow>
                    <TableCell>
                      <code className="text-xs">
                        scryer.lang_matches(code, pattern)
                      </code>
                    </TableCell>
                    <TableCell>
                      <code className="rounded bg-muted px-1 py-0.5 text-xs text-muted-foreground">
                        bool
                      </code>
                    </TableCell>
                    <TableCell className="text-muted-foreground">
                      {t("settings.refFnLangMatches")}
                    </TableCell>
                  </TableRow>
                  <TableRow>
                    <TableCell>
                      <code className="text-xs">
                        scryer.normalize_source(raw)
                      </code>
                    </TableCell>
                    <TableCell>
                      <code className="rounded bg-muted px-1 py-0.5 text-xs text-muted-foreground">
                        string
                      </code>
                    </TableCell>
                    <TableCell className="text-muted-foreground">
                      {t("settings.refFnNormalizeSource")}
                    </TableCell>
                  </TableRow>
                  <TableRow>
                    <TableCell>
                      <code className="text-xs">
                        scryer.normalize_codec(raw)
                      </code>
                    </TableCell>
                    <TableCell>
                      <code className="rounded bg-muted px-1 py-0.5 text-xs text-muted-foreground">
                        string
                      </code>
                    </TableCell>
                    <TableCell className="text-muted-foreground">
                      {t("settings.refFnNormalizeCodec")}
                    </TableCell>
                  </TableRow>
                </TableBody>
              </Table>
            </div>
          </div>

          <div>
            <h4 className="mb-1 font-semibold">
              {t("settings.refSectionOutput")}
            </h4>
            <p className="mb-2 text-muted-foreground">
              {t("settings.refOutputIntro")}
            </p>
            <pre className="rounded border border-border bg-muted/50 p-3 text-xs leading-relaxed">
              {`package scryer.rules.user.<rule_id>
import rego.v1

# Return a map of score codes to point deltas.
# Positive values boost the release, negative values penalize it.
# Use scryer.block_score() to hard-block a release.

score_entry["dual_audio_bonus"] := 500 if {
    input.release.is_dual_audio
}

score_entry["too_old"] := scryer.block_score() if {
    input.release.age_days > 365
}

score_entry["too_few_chapters"] := scryer.block_score() if {
    input.file != null
    input.file.num_chapters < 2
}

score_entry["japanese_audio_bonus"] := 300 if {
    input.file != null
    some lang in input.file.audio_languages
    scryer.lang_matches(lang, "ja")
}`}
            </pre>
          </div>
        </CardContent>
      ) : null}
    </Card>
  );
}

/**
 * Parse a managed-rule key like "convenience:required-audio:anime"
 * into a human-readable label like "Required Audio · Anime".
 */
function managedRuleLabel(key: string): string {
  const parts = key.split(":");
  // Skip the first segment ("convenience") — it's the namespace, not useful to show.
  const labelParts = parts.slice(1).map((segment) =>
    segment
      .split("-")
      .map((w) => w.charAt(0).toUpperCase() + w.slice(1))
      .join(" "),
  );
  return labelParts.join(" \u00B7 ");
}

function ManagedBadge({ managedKey }: { managedKey: string }) {
  return (
    <Badge tone="info" className="ml-2 rounded-full text-[10px]">
      {managedRuleLabel(managedKey)}
    </Badge>
  );
}

function TrashLocalePacksCard({
  ruleSetRecords,
  mutatingRuleSetId,
  toggleRuleSetEnabled,
}: {
  ruleSetRecords: RuleSetRecord[];
  mutatingRuleSetId: string | null;
  toggleRuleSetEnabled: (record: RuleSetRecord) => Promise<void> | void;
}) {
  const t = useTranslate();
  const packs = trashLocalePacks(ruleSetRecords);
  if (packs.length === 0) return null;
  const enabledCount = packs.filter((record) => record.enabled).length;

  return (
    <Collapsible id="settings-trash-packs" defaultOpen={false}>
      <Card>
        <CollapsibleTrigger
          type="button"
          className="group w-full text-left outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2"
        >
          <CardHeader className="flex-row items-center gap-3">
            <div className="min-w-0 flex-1">
              <CardTitle className="text-base">{t("settings.trashPacksTitle")}</CardTitle>
              <p className="mt-1 text-xs text-muted-foreground">
                {t("settings.trashPacksHelp")}
              </p>
            </div>
            <Badge tone="neutral">
              {enabledCount}/{packs.length} {t("label.enabled")}
            </Badge>
            <ChevronDown
              aria-hidden="true"
              className="h-4 w-4 shrink-0 text-muted-foreground transition-transform group-data-[state=open]:rotate-180"
            />
          </CardHeader>
        </CollapsibleTrigger>
        <CollapsibleContent>
          <CardContent className="space-y-2 border-t border-border">
            {packs.map((record) => (
              <div
                key={record.id}
                id={selectorId("settings-trash-pack-row", record.managedKey ?? record.id)}
                className="flex items-start gap-3 rounded-md border border-border p-3"
              >
                <Checkbox
                  id={selectorId(
                    "settings-trash-pack-toggle",
                    record.managedKey ?? record.id,
                  )}
                  checked={record.enabled}
                  onCheckedChange={() => void toggleRuleSetEnabled(record)}
                  disabled={mutatingRuleSetId === record.id}
                  aria-label={`${t("label.enabled")}: ${record.name}`}
                />
                <Label
                  htmlFor={selectorId(
                    "settings-trash-pack-toggle",
                    record.managedKey ?? record.id,
                  )}
                  className="min-w-0 cursor-pointer"
                >
                  <span className="block font-medium">{record.name}</span>
                  <span className="mt-1 block text-xs font-normal text-muted-foreground">
                    {record.description}
                  </span>
                </Label>
              </div>
            ))}
          </CardContent>
        </CollapsibleContent>
      </Card>
    </Collapsible>
  );
}

function FacetBadges({ facets }: { facets: string[] }) {
  if (facets.length === 0) {
    return (
      <Badge tone="info" className="capitalize">
        Global
      </Badge>
    );
  }
  return (
    <div className="flex gap-1">
      {facets.map((f) => (
        <Badge key={f} tone="neutral" className="capitalize">
          {f}
        </Badge>
      ))}
    </div>
  );
}

type CommunityPack = {
  id: string;
  name: string;
  description: string;
  author: string;
  version: string;
};
type CommunityTemplate = {
  id: string;
  title: string;
  description: string;
  category: string;
  regoSource: string;
  appliedFacets: string[];
};

function RuleLibrary({
  onApply,
  defaultOpen,
}: {
  onApply: (template: RuleTemplate) => void;
  defaultOpen?: boolean;
}) {
  const t = useTranslate();
  const client = useClient();
  const [open, setOpen] = React.useState(defaultOpen ?? false);
  const [tab, setTab] = React.useState<"builtin" | "community">("builtin");
  const [categoryFilter, setCategoryFilter] = React.useState<string>("All");

  // Community state
  const [communityPacks, setCommunityPacks] = React.useState<CommunityPack[]>(
    [],
  );
  const [communityPacksLoaded, setCommunityPacksLoaded] = React.useState(false);
  const [selectedPack, setSelectedPack] = React.useState<CommunityPack | null>(
    null,
  );
  const [packTemplates, setPackTemplates] = React.useState<CommunityTemplate[]>(
    [],
  );
  const [packLoading, setPackLoading] = React.useState(false);

  React.useEffect(() => {
    if (tab === "community" && !communityPacksLoaded) {
      client
        .query(rulePackRegistryQuery, {})
        .toPromise()
        .then(({ data }) => {
          setCommunityPacks(data?.rulePackRegistry ?? []);
          setCommunityPacksLoaded(true);
        })
        .catch(() => {
          setCommunityPacksLoaded(true);
        });
    }
  }, [tab, communityPacksLoaded, client]);

  const loadPack = React.useCallback(
    async (pack: CommunityPack) => {
      setSelectedPack(pack);
      setPackLoading(true);
      try {
        const { data } = await client
          .query(rulePackTemplatesQuery, { packId: pack.id })
          .toPromise();
        setPackTemplates(data?.rulePackTemplates ?? []);
      } catch {
        setPackTemplates([]);
      } finally {
        setPackLoading(false);
      }
    },
    [client],
  );

  const filtered =
    categoryFilter === "All"
      ? RULE_TEMPLATES
      : RULE_TEMPLATES.filter((tpl) => tpl.category === categoryFilter);

  return (
    <Card>
      <CardHeader
        className="cursor-pointer select-none"
        onClick={() => setOpen((prev) => !prev)}
      >
        <CardTitle className="flex items-center gap-2 text-base">
          <Library className="h-4 w-4" />
          {t("settings.ruleLibrary")}
          <ChevronDown
            className={`ml-auto h-4 w-4 transition-transform ${open ? "rotate-180" : ""}`}
          />
        </CardTitle>
        <p className="text-xs text-muted-foreground">
          {t("settings.ruleLibraryDescription")}
        </p>
      </CardHeader>
      {open ? (
        <CardContent>
          {/* Tab selector */}
          <div className="mb-3 flex gap-2 border-b border-border pb-2">
            <button
              id="settings-rules-library-tab-builtin"
              type="button"
              className={cn(
                "px-3 py-1.5 text-sm font-medium transition-colors rounded-t",
                tab === "builtin"
                  ? "text-primary border-b-2 border-primary"
                  : "text-muted-foreground hover:text-foreground",
              )}
              onClick={() => setTab("builtin")}
            >
              Built-in
            </button>
            <button
              id="settings-rules-library-tab-community"
              type="button"
              className={cn(
                "px-3 py-1.5 text-sm font-medium transition-colors rounded-t",
                tab === "community"
                  ? "text-primary border-b-2 border-primary"
                  : "text-muted-foreground hover:text-foreground",
              )}
              onClick={() => setTab("community")}
            >
              Community
            </button>
          </div>

          {tab === "builtin" ? (
            <>
              <div className="mb-3 flex flex-wrap gap-1">
                {[
                  t("settings.ruleLibraryAll"),
                  ...RULE_TEMPLATE_CATEGORIES,
                ].map((cat) => {
                  const isAll = cat === t("settings.ruleLibraryAll");
                  const active = isAll
                    ? categoryFilter === "All"
                    : categoryFilter === cat;
                  return (
                    <button
                      id={selectorId("settings-rules-library-category", isAll ? "all" : cat)}
                      key={cat}
                      type="button"
                      className={cn(
                        "rounded-full px-3 py-1 text-xs font-medium transition-colors",
                        active
                          ? "bg-primary text-primary-foreground"
                          : "bg-muted text-muted-foreground hover:bg-muted/80",
                      )}
                      onClick={() => setCategoryFilter(isAll ? "All" : cat)}
                    >
                      {cat}
                    </button>
                  );
                })}
              </div>
              <TemplateGrid
                templates={filtered}
                onApply={(tpl) => {
                  onApply(tpl);
                  setOpen(false);
                }}
              />
            </>
          ) : /* Community tab */
          selectedPack ? (
            <div>
              <button
                id="settings-rules-library-back-to-packs"
                type="button"
                className="mb-3 text-xs text-muted-foreground hover:text-foreground"
                onClick={() => {
                  setSelectedPack(null);
                  setPackTemplates([]);
                }}
              >
                &larr; Back to packs
              </button>
              <p className="mb-1 text-sm font-medium">{selectedPack.name}</p>
              <p className="mb-3 text-xs text-muted-foreground">
                {selectedPack.description}
              </p>
              {packLoading ? (
                <p className="text-sm text-muted-foreground">
                  {t("label.loading")}
                </p>
              ) : (
                <TemplateGrid
                  templates={packTemplates.map((t) => ({
                    id: t.id,
                    title: t.title,
                    description: t.description,
                    category: t.category,
                    regoSource: t.regoSource,
                    appliedFacets: t.appliedFacets,
                  }))}
                  onApply={(tpl) => {
                    onApply(tpl);
                    setOpen(false);
                  }}
                />
              )}
            </div>
          ) : (
            <div className="grid grid-cols-1 gap-2 @[420px]:grid-cols-2 @[640px]:grid-cols-3">
              {communityPacks.length === 0 && communityPacksLoaded ? (
                <p className="col-span-full text-sm text-muted-foreground">
                  {t("settings.ruleLibraryCommunityEmpty")}
                </p>
              ) : null}
              {communityPacks.map((pack) => (
                <button
                  id={selectorId("settings-rules-library-pack", pack.id)}
                  key={pack.id}
                  type="button"
                  className="group rounded-lg border border-border bg-card/50 p-3 text-left transition-colors hover:border-primary/40 hover:bg-card"
                  onClick={() => void loadPack(pack)}
                >
                  <p className="text-sm font-medium text-foreground group-hover:text-primary">
                    {pack.name}
                  </p>
                  <p className="mt-1 text-xs text-muted-foreground line-clamp-2">
                    {pack.description}
                  </p>
                  <div className="mt-2 flex items-center gap-2">
                    <Badge tone="info" className="text-[10px]">
                      {pack.author}
                    </Badge>
                    <span className="text-[10px] text-muted-foreground">
                      v{pack.version}
                    </span>
                  </div>
                </button>
              ))}
            </div>
          )}
        </CardContent>
      ) : null}
    </Card>
  );
}

function TemplateGrid({
  templates,
  onApply,
}: {
  templates: RuleTemplate[];
  onApply: (template: RuleTemplate) => void;
}) {
  return (
    <div className="grid grid-cols-1 gap-2 @[420px]:grid-cols-2 @[640px]:grid-cols-3">
      {templates.map((tpl) => (
        <button
          id={selectorId("settings-rules-library-template", tpl.id)}
          key={tpl.id}
          type="button"
          className="group rounded-lg border border-border bg-card/50 p-3 text-left transition-colors hover:border-primary/40 hover:bg-card"
          onClick={() => onApply(tpl)}
        >
          <p className="text-sm font-medium text-foreground group-hover:text-primary">
            {tpl.title}
          </p>
          <p className="mt-1 text-xs text-muted-foreground line-clamp-2">
            {tpl.description}
          </p>
          <div className="mt-2 flex items-center gap-2">
            <Badge tone="neutral" className="text-[10px]">
              {tpl.category}
            </Badge>
            {tpl.appliedFacets
              ?.filter((f) => f.toLowerCase() !== tpl.category.toLowerCase())
              .map((f) => (
                <Badge key={f} tone="info" className="text-[10px]">
                  {f}
                </Badge>
              ))}
          </div>
        </button>
      ))}
    </div>
  );
}

export function SettingsRulesSection({
  isEditorOpen,
  editorMode,
  editingRuleSetId,
  ruleSetDraft,
  setRuleSetDraft,
  submitRuleSet,
  mutatingRuleSetId,
  resetRuleSetDraft,
  startCreateRuleSet,
  ruleSetRecords,
  copyRuleSet,
  editRuleSet,
  toggleRuleSetEnabled,
  saveManagedTagFilter,
  deleteRuleSet,
  validateDraft,
  validating,
  validationResult,
  applyTemplate,
}: SettingsRulesSectionProps) {
  const t = useTranslate();
  const validationDiagnostics = React.useMemo(
    () => getRuleValidationDiagnostics(validationResult),
    [validationResult],
  );

  return (
    <div id="settings-rules-section" className="space-y-4 text-sm">
      <div className="mx-auto flex w-full max-w-[2176px] flex-col gap-4 xl:flex-row xl:items-start">
        <div className="min-w-0 flex-1">
          <div className="mx-auto w-full max-w-[1280px] space-y-4">
      <TrashLocalePacksCard
        ruleSetRecords={ruleSetRecords}
        mutatingRuleSetId={mutatingRuleSetId}
        toggleRuleSetEnabled={toggleRuleSetEnabled}
        saveManagedTagFilter={saveManagedTagFilter}
      />
      <div className="rounded border border-border">
        <div className="flex items-center justify-between border-b border-border px-3 py-2">
          <CardTitle className="text-base">
            {t("settings.rules")}
          </CardTitle>
        </div>
        <div className="overflow-x-auto">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>{t("label.name")}</TableHead>
                <TableHead>{t("settings.ruleDescription")}</TableHead>
                <TableHead>{t("settings.ruleAppliedFacets")}</TableHead>
                <TableHead className="text-center">
                  {t("settings.rulePriority")}
                </TableHead>
                <TableHead className="text-center">
                  {t("label.enabled")}
                </TableHead>
                <TableHead className="text-right">
                  {t("label.actions")}
                </TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {ruleSetRecords.filter(isUserOwnedRuleSet).map((record) => (
                <TableRow
                  key={record.id}
                  id={selectorId("settings-rule-row", record.id)}
                >
                  <TableCell className="font-medium">
                    <span id={selectorId("settings-rule-name", record.name)}>
                      {record.name}
                    </span>
                    {record.isManaged && record.managedKey ? (
                      <ManagedBadge managedKey={record.managedKey} />
                    ) : null}
                  </TableCell>
                  <TableCell className="max-w-[200px] truncate text-muted-foreground">
                    {record.description || "—"}
                  </TableCell>
                  <TableCell>
                    <FacetBadges facets={record.appliedFacets} />
                  </TableCell>
                  <TableCell className="text-center">
                    {record.priority}
                  </TableCell>
                  <TableCell className="text-center">
                    <RenderBooleanIcon
                      value={record.enabled}
                      label={`${t("label.enabled")}: ${record.name}`}
                    />
                  </TableCell>
                  <TableCell className="text-right">
                    <div className="flex justify-end gap-1">
                      <IconButton
                        id={selectorId("settings-rule-toggle", record.id)}
                        label={
                          record.enabled
                            ? t("label.disable")
                            : t("label.enable")
                        }
                        tone={record.enabled ? "disabled" : "enabled"}
                        onClick={() => void toggleRuleSetEnabled(record)}
                        disabled={mutatingRuleSetId === record.id}
                      >
                        <Power className="h-4 w-4" />
                      </IconButton>
                      {record.isManaged ? (
                        <IconButton
                          id={selectorId("settings-rule-copy", record.id)}
                          label={t("settings.ruleCopyAsCustom")}
                          tone="neutral"
                          onClick={() => copyRuleSet(record)}
                        >
                          <Copy className="h-4 w-4" />
                        </IconButton>
                      ) : isUserOwnedRuleSet(record) ? (
                        <>
                          <IconButton
                            id={selectorId("settings-rule-edit", record.id)}
                            label={t("label.edit")}
                            tone="edit"
                            onClick={() => editRuleSet(record)}
                          >
                            <Edit className="h-4 w-4" />
                          </IconButton>
                          <IconButton
                            id={selectorId("settings-rule-delete", record.id)}
                            label={t("label.delete")}
                            tone="delete"
                            onClick={() => void deleteRuleSet(record)}
                            disabled={mutatingRuleSetId === record.id}
                          >
                            <Trash2 className="h-4 w-4" />
                          </IconButton>
                        </>
                      ) : null}
                    </div>
                  </TableCell>
                </TableRow>
              ))}
              {ruleSetRecords.length === 0 ? (
                <TableRow>
                  <TableCell colSpan={6} className="text-muted-foreground">
                    {t("settings.noRulesFound")}
                  </TableCell>
                </TableRow>
              ) : null}
            </TableBody>
          </Table>
        </div>
      </div>

      {isEditorOpen ? (
        <>
          <Card>
            <CardHeader>
              <CardTitle className="text-base">
                {editingRuleSetId
                  ? t("settings.ruleUpdate")
                  : t("settings.ruleCreate")}
              </CardTitle>
            </CardHeader>
            <CardContent>
              <form id="settings-rule-form" className="space-y-3" onSubmit={submitRuleSet}>
                <div className="grid gap-3 md:grid-cols-3">
                  <label>
                    <Label className="mb-2 block">{t("label.name")}</Label>
                    <Input
                      id="settings-rule-name"
                      value={ruleSetDraft.name}
                      onChange={(e) =>
                        setRuleSetDraft((prev) => ({
                          ...prev,
                          name: e.target.value,
                        }))
                      }
                      required
                      placeholder="my_rule"
                    />
                  </label>
                  <label>
                    <Label className="mb-2 block">
                      {t("settings.ruleDescription")}
                    </Label>
                    <Input
                      id="settings-rule-description"
                      value={ruleSetDraft.description}
                      onChange={(e) =>
                        setRuleSetDraft((prev) => ({
                          ...prev,
                          description: e.target.value,
                        }))
                      }
                      placeholder="Block releases over 100 GiB"
                    />
                  </label>
                  <label>
                    <Label className="mb-2 block">
                      {t("settings.rulePriority")}
                    </Label>
                    <Input
                      id="settings-rule-priority"
                      {...integerInputProps}
                      value={ruleSetDraft.priority}
                      onChange={(e) =>
                        setRuleSetDraft((prev) => ({
                          ...prev,
                          priority: Number(sanitizeDigits(e.target.value)) || 0,
                        }))
                      }
                      placeholder="0"
                    />
                  </label>
                </div>

                <div>
                  <Label className="mb-2 block">
                    {t("settings.ruleRegoSource")}
                  </Label>
                  <LazyRegoEditor
                    value={ruleSetDraft.regoSource}
                    onChange={(value) =>
                      setRuleSetDraft((prev) => ({
                        ...prev,
                        regoSource: value,
                      }))
                    }
                    diagnostics={validationDiagnostics}
                    minLines={10}
                    maxLines={35}
                  />
                </div>

                <div>
                  <Label className="mb-2 block">
                    {t("settings.ruleAppliedFacets")}
                  </Label>
                  <p className="mb-2 text-xs text-muted-foreground">
                    {t("settings.ruleAppliedFacetsHelp")}
                  </p>
                  <div className="flex items-center gap-4">
                    {FACET_OPTIONS.map((opt) => (
                      <label
                        key={opt.value}
                        className="flex items-center gap-2"
                      >
                        <Checkbox
                          id={selectorId("settings-rule-facet", opt.value)}
                          checked={ruleSetDraft.appliedFacets.includes(
                            opt.value,
                          )}
                          onCheckedChange={(value) => {
                            setRuleSetDraft((prev) => {
                              const next =
                                value === true
                                  ? [...prev.appliedFacets, opt.value]
                                  : prev.appliedFacets.filter(
                                      (f) => f !== opt.value,
                                    );
                              return { ...prev, appliedFacets: next };
                            });
                          }}
                        />
                        <span className="text-sm">{opt.label}</span>
                      </label>
                    ))}
                  </div>
                </div>

                <label className="flex items-center gap-2">
                  <Checkbox
                    id="settings-rule-enabled"
                    checked={ruleSetDraft.enabled}
                    onCheckedChange={(value) =>
                      setRuleSetDraft((prev) => ({
                        ...prev,
                        enabled: value === true,
                      }))
                    }
                  />
                  <span className="text-sm">{t("label.enabled")}</span>
                </label>

                {validationResult ? (
                  <div
                    className={`rounded border px-3 py-2 text-sm ${
                      validationResult.valid
                        ? "border-[var(--scry-success-border)] bg-[var(--scry-success-bg)] text-[var(--scry-success-text)]"
                        : "border-[var(--scry-danger-border)] bg-[var(--scry-danger-bg)] text-[var(--scry-danger-text)]"
                    }`}
                  >
                    {validationResult.valid ? (
                      t("settings.ruleValid")
                    ) : (
                      <div className="space-y-2">
                        {validationResult.errors.map((err, i) => (
                          <pre
                            key={i}
                            className="overflow-x-auto whitespace-pre rounded-[9px] border border-[var(--scry-danger-border)] bg-[var(--scry-danger-bg)] p-3 font-mono font-[var(--font-code)] text-[12px] leading-5 text-[var(--scry-danger-text)]"
                          >
                            {formatRuleValidationError(err)}
                          </pre>
                        ))}
                      </div>
                    )}
                  </div>
                ) : null}

                <div className="flex gap-2">
                  <Button id="settings-rule-save" type="submit" disabled={mutatingRuleSetId !== null}>
                    {mutatingRuleSetId !== null
                      ? t("label.saving")
                      : editingRuleSetId
                        ? t("settings.ruleUpdate")
                        : t("settings.ruleCreate")}
                  </Button>
                  <Button
                    id="settings-rule-validate"
                    type="button"
                    variant="secondary"
                    onClick={() => void validateDraft()}
                    disabled={validating || !ruleSetDraft.regoSource.trim()}
                  >
                    {validating
                      ? t("settings.ruleValidating")
                      : t("settings.ruleValidate")}
                  </Button>
                  <Button
                    id="settings-rule-cancel"
                    type="button"
                    variant="secondary"
                    onClick={resetRuleSetDraft}
                  >
                    {t("label.cancel")}
                  </Button>
                </div>
              </form>
            </CardContent>
          </Card>
          {editorMode === "edit" ? (
            <div className="flex justify-center">
              <AddNewButton
                id="settings-rule-create-new"
                icon={Plus}
                label={t("settings.ruleCreateNew")}
                onClick={startCreateRuleSet}
                disabled={mutatingRuleSetId !== null}
              />
            </div>
          ) : null}
        </>
      ) : (
        <div className="flex justify-center">
          <AddNewButton
            id="settings-rule-create"
            icon={Plus}
            label={t("settings.ruleCreateNew")}
            onClick={startCreateRuleSet}
            disabled={mutatingRuleSetId !== null}
          />
        </div>
      )}
          </div>
        </div>
        <div className="@container w-full space-y-4 xl:w-[44%] xl:max-w-[880px] xl:shrink-0">
          <RuleLibrary
            defaultOpen={ruleSetRecords.length === 0}
            onApply={applyTemplate}
          />
          <RulesContextReference />
        </div>
      </div>
    </div>
  );
}
