import * as React from "react";
import { BookOpen, ChevronDown, Edit, FileCode2, Power, Trash2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { LazyRegoEditor } from "@/components/common/lazy-rego-editor";
import { RenderBooleanIcon } from "@/components/common/boolean-icon";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import type { Translate } from "@/components/root/types";
import type { RuleSetRecord, RuleSetDraft, RuleValidationResult } from "@/lib/types/rule-sets";

type SettingsRulesSectionProps = {
  t: Translate;
  editingRuleSetId: string | null;
  ruleSetDraft: RuleSetDraft;
  setRuleSetDraft: React.Dispatch<React.SetStateAction<RuleSetDraft>>;
  submitRuleSet: (event: React.FormEvent<HTMLFormElement>) => Promise<void> | void;
  mutatingRuleSetId: string | null;
  resetRuleSetDraft: () => void;
  ruleSetRecords: RuleSetRecord[];
  editRuleSet: (record: RuleSetRecord) => void;
  toggleRuleSetEnabled: (record: RuleSetRecord) => Promise<void> | void;
  deleteRuleSet: (record: RuleSetRecord) => Promise<void> | void;
  validateDraft: () => Promise<void> | void;
  validating: boolean;
  validationResult: RuleValidationResult | null;
};

const FACET_OPTIONS = [
  { value: "movie", label: "Movie" },
  { value: "tv", label: "TV" },
  { value: "anime", label: "Anime" },
];

type RefField = { field: string; type: string; descKey: string };

const RELEASE_FIELDS: RefField[] = [
  { field: "raw_title", type: "string", descKey: "settings.refReleaseRawTitle" },
  { field: "quality", type: "string?", descKey: "settings.refReleaseQuality" },
  { field: "source", type: "string?", descKey: "settings.refReleaseSource" },
  { field: "video_codec", type: "string?", descKey: "settings.refReleaseVideoCodec" },
  { field: "audio", type: "string?", descKey: "settings.refReleaseAudio" },
  { field: "audio_codecs", type: "string[]", descKey: "settings.refReleaseAudioCodecs" },
  { field: "audio_channels", type: "string?", descKey: "settings.refReleaseAudioChannels" },
  { field: "languages_audio", type: "string[]", descKey: "settings.refReleaseLangsAudio" },
  { field: "languages_subtitles", type: "string[]", descKey: "settings.refReleaseLangsSub" },
  { field: "is_dual_audio", type: "bool", descKey: "settings.refReleaseIsDualAudio" },
  { field: "is_atmos", type: "bool", descKey: "settings.refReleaseIsAtmos" },
  { field: "is_dolby_vision", type: "bool", descKey: "settings.refReleaseIsDV" },
  { field: "detected_hdr", type: "bool", descKey: "settings.refReleaseDetectedHdr" },
  { field: "is_remux", type: "bool", descKey: "settings.refReleaseIsRemux" },
  { field: "is_bd_disk", type: "bool", descKey: "settings.refReleaseIsBdDisk" },
  { field: "is_proper_upload", type: "bool", descKey: "settings.refReleaseIsProper" },
  { field: "release_group", type: "string?", descKey: "settings.refReleaseGroup" },
  { field: "year", type: "number?", descKey: "settings.refReleaseYear" },
  { field: "parse_confidence", type: "float", descKey: "settings.refReleaseParseConf" },
  { field: "size_bytes", type: "number?", descKey: "settings.refReleaseSizeBytes" },
  { field: "age_days", type: "number?", descKey: "settings.refReleaseAgeDays" },
  { field: "thumbs_up", type: "number?", descKey: "settings.refReleaseThumbsUp" },
  { field: "thumbs_down", type: "number?", descKey: "settings.refReleaseThumbsDown" },
];

const PROFILE_FIELDS: RefField[] = [
  { field: "id", type: "string", descKey: "settings.refProfileId" },
  { field: "name", type: "string", descKey: "settings.refProfileName" },
  { field: "quality_tiers", type: "string[]", descKey: "settings.refProfileQualityTiers" },
  { field: "archival_quality", type: "string?", descKey: "settings.refProfileArchivalQuality" },
  { field: "allow_unknown_quality", type: "bool", descKey: "settings.refProfileAllowUnknown" },
  { field: "source_allowlist", type: "string[]", descKey: "settings.refProfileSourceAllow" },
  { field: "source_blocklist", type: "string[]", descKey: "settings.refProfileSourceBlock" },
  { field: "video_codec_allowlist", type: "string[]", descKey: "settings.refProfileVCodecAllow" },
  { field: "video_codec_blocklist", type: "string[]", descKey: "settings.refProfileVCodecBlock" },
  { field: "audio_codec_allowlist", type: "string[]", descKey: "settings.refProfileACodecAllow" },
  { field: "audio_codec_blocklist", type: "string[]", descKey: "settings.refProfileACodecBlock" },
  { field: "atmos_preferred", type: "bool", descKey: "settings.refProfileAtmosPreferred" },
  { field: "dolby_vision_allowed", type: "bool", descKey: "settings.refProfileDVAllowed" },
  { field: "detected_hdr_allowed", type: "bool", descKey: "settings.refProfileHdrAllowed" },
  { field: "prefer_remux", type: "bool", descKey: "settings.refProfilePreferRemux" },
  { field: "allow_bd_disk", type: "bool", descKey: "settings.refProfileAllowBdDisk" },
  { field: "allow_upgrades", type: "bool", descKey: "settings.refProfileAllowUpgrades" },
  { field: "prefer_dual_audio", type: "bool", descKey: "settings.refProfilePreferDualAudio" },
  { field: "required_audio_languages", type: "string[]", descKey: "settings.refProfileRequiredLangs" },
];

const CONTEXT_FIELDS: RefField[] = [
  { field: "title_id", type: "string?", descKey: "settings.refCtxTitleId" },
  { field: "media_type", type: "string", descKey: "settings.refCtxMediaType" },
  { field: "category", type: "string", descKey: "settings.refCtxCategory" },
  { field: "tags", type: "string[]", descKey: "settings.refCtxTags" },
  { field: "has_existing_file", type: "bool", descKey: "settings.refCtxHasExisting" },
  { field: "existing_score", type: "number?", descKey: "settings.refCtxExistingScore" },
  { field: "search_mode", type: "string", descKey: "settings.refCtxSearchMode" },
  { field: "runtime_minutes", type: "number?", descKey: "settings.refCtxRuntimeMin" },
  { field: "is_anime", type: "bool", descKey: "settings.refCtxIsAnime" },
  { field: "is_filler", type: "bool", descKey: "settings.refCtxIsFiller" },
];

const BUILTIN_SCORE_FIELDS: RefField[] = [
  { field: "total", type: "number", descKey: "settings.refBuiltinTotal" },
  { field: "blocked", type: "bool", descKey: "settings.refBuiltinBlocked" },
  { field: "codes", type: "string[]", descKey: "settings.refBuiltinCodes" },
];

type RefSectionDef = { titleKey: string; path: string; fields: RefField[] };

const REF_SECTIONS: RefSectionDef[] = [
  { titleKey: "settings.refSectionRelease", path: "input.release", fields: RELEASE_FIELDS },
  { titleKey: "settings.refSectionProfile", path: "input.profile", fields: PROFILE_FIELDS },
  { titleKey: "settings.refSectionContext", path: "input.context", fields: CONTEXT_FIELDS },
  { titleKey: "settings.refSectionBuiltinScore", path: "input.builtin_score", fields: BUILTIN_SCORE_FIELDS },
];

function RefFieldTable({ t, section }: { t: Translate; section: RefSectionDef }) {
  return (
    <div>
      <h4 className="mb-1 font-semibold">
        <code className="rounded bg-muted px-1.5 py-0.5 text-xs">{section.path}</code>
        {" "}<span className="text-muted-foreground font-normal">{t(section.titleKey)}</span>
      </h4>
      <div className="overflow-x-auto">
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead className="w-[220px]">{t("settings.refColField")}</TableHead>
              <TableHead className="w-[100px]">{t("settings.refColType")}</TableHead>
              <TableHead>{t("settings.refColDescription")}</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {section.fields.map((f) => (
              <TableRow key={f.field}>
                <TableCell>
                  <code className="text-xs">{section.path}.{f.field}</code>
                </TableCell>
                <TableCell>
                  <code className="rounded bg-muted px-1 py-0.5 text-xs text-muted-foreground">{f.type}</code>
                </TableCell>
                <TableCell className="text-muted-foreground">{t(f.descKey)}</TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      </div>
    </div>
  );
}

function RulesContextReference({ t }: { t: Translate }) {
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
          <ChevronDown className={`ml-auto h-4 w-4 transition-transform ${open ? "rotate-180" : ""}`} />
        </CardTitle>
        <p className="text-xs text-muted-foreground">{t("settings.refSubtitle")}</p>
      </CardHeader>
      {open ? (
        <CardContent className="space-y-6 text-sm">
          <p className="text-muted-foreground">{t("settings.refIntro")}</p>

          <div>
            <h4 className="mb-2 font-semibold">{t("settings.refSectionSandbox")}</h4>
            <p className="mb-2 text-muted-foreground">{t("settings.refSandboxIntro")}</p>
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
            <RefFieldTable key={section.path} t={t} section={section} />
          ))}

          <div>
            <h4 className="mb-1 font-semibold">{t("settings.refSectionBuiltins")}</h4>
            <p className="mb-2 text-muted-foreground">{t("settings.refBuiltinsIntro")}</p>
            <div className="overflow-x-auto">
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead className="w-[280px]">{t("settings.refColFunction")}</TableHead>
                    <TableHead className="w-[100px]">{t("settings.refColReturns")}</TableHead>
                    <TableHead>{t("settings.refColDescription")}</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  <TableRow>
                    <TableCell><code className="text-xs">scryer.block_score()</code></TableCell>
                    <TableCell><code className="rounded bg-muted px-1 py-0.5 text-xs text-muted-foreground">number</code></TableCell>
                    <TableCell className="text-muted-foreground">{t("settings.refFnBlockScore")}</TableCell>
                  </TableRow>
                  <TableRow>
                    <TableCell><code className="text-xs">scryer.size_gib(bytes)</code></TableCell>
                    <TableCell><code className="rounded bg-muted px-1 py-0.5 text-xs text-muted-foreground">float</code></TableCell>
                    <TableCell className="text-muted-foreground">{t("settings.refFnSizeGib")}</TableCell>
                  </TableRow>
                  <TableRow>
                    <TableCell><code className="text-xs">scryer.lang_matches(code, pattern)</code></TableCell>
                    <TableCell><code className="rounded bg-muted px-1 py-0.5 text-xs text-muted-foreground">bool</code></TableCell>
                    <TableCell className="text-muted-foreground">{t("settings.refFnLangMatches")}</TableCell>
                  </TableRow>
                  <TableRow>
                    <TableCell><code className="text-xs">scryer.normalize_source(raw)</code></TableCell>
                    <TableCell><code className="rounded bg-muted px-1 py-0.5 text-xs text-muted-foreground">string</code></TableCell>
                    <TableCell className="text-muted-foreground">{t("settings.refFnNormalizeSource")}</TableCell>
                  </TableRow>
                  <TableRow>
                    <TableCell><code className="text-xs">scryer.normalize_codec(raw)</code></TableCell>
                    <TableCell><code className="rounded bg-muted px-1 py-0.5 text-xs text-muted-foreground">string</code></TableCell>
                    <TableCell className="text-muted-foreground">{t("settings.refFnNormalizeCodec")}</TableCell>
                  </TableRow>
                </TableBody>
              </Table>
            </div>
          </div>

          <div>
            <h4 className="mb-1 font-semibold">{t("settings.refSectionOutput")}</h4>
            <p className="mb-2 text-muted-foreground">{t("settings.refOutputIntro")}</p>
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

score_entry["large_file_penalty"] := -200 if {
    scryer.size_gib(input.release.size_bytes) > 80
}

score_entry["japanese_audio_bonus"] := 300 if {
    some lang in input.release.languages_audio
    scryer.lang_matches(lang, "ja")
}`}
            </pre>
          </div>
        </CardContent>
      ) : null}
    </Card>
  );
}

function FacetBadges({ facets }: { facets: string[] }) {
  if (facets.length === 0) {
    return <span className="rounded bg-blue-900/40 px-1.5 py-0.5 text-xs text-blue-300">Global</span>;
  }
  return (
    <div className="flex gap-1">
      {facets.map((f) => (
        <span key={f} className="rounded bg-muted px-1.5 py-0.5 text-xs text-muted-foreground capitalize">
          {f}
        </span>
      ))}
    </div>
  );
}

export function SettingsRulesSection({
  t,
  editingRuleSetId,
  ruleSetDraft,
  setRuleSetDraft,
  submitRuleSet,
  mutatingRuleSetId,
  resetRuleSetDraft,
  ruleSetRecords,
  editRuleSet,
  toggleRuleSetEnabled,
  deleteRuleSet,
  validateDraft,
  validating,
  validationResult,
}: SettingsRulesSectionProps) {
  return (
    <div className="space-y-4 text-sm">
      <CardTitle className="flex items-center gap-2 text-base">
        <FileCode2 className="h-4 w-4" />
        {t("settings.rulesSection")}
      </CardTitle>

      <div className="rounded border border-border">
        <div className="flex items-center justify-between border-b border-border px-3 py-2">
          <CardTitle className="text-base">{t("settings.existingRules")}</CardTitle>
        </div>
        <div className="overflow-x-auto">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>{t("settings.ruleName")}</TableHead>
                <TableHead>{t("settings.ruleDescription")}</TableHead>
                <TableHead>{t("settings.ruleAppliedFacets")}</TableHead>
                <TableHead className="text-center">{t("settings.rulePriority")}</TableHead>
                <TableHead className="text-center">{t("label.enabled")}</TableHead>
                <TableHead className="text-right">{t("settings.actions")}</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {ruleSetRecords.map((record) => (
                <TableRow key={record.id}>
                  <TableCell className="font-medium">{record.name}</TableCell>
                  <TableCell className="max-w-[200px] truncate text-muted-foreground">
                    {record.description || "—"}
                  </TableCell>
                  <TableCell>
                    <FacetBadges facets={record.appliedFacets} />
                  </TableCell>
                  <TableCell className="text-center">{record.priority}</TableCell>
                  <TableCell className="text-center">
                    <RenderBooleanIcon
                      value={record.enabled}
                      label={`${t("label.enabled")}: ${record.name}`}
                    />
                  </TableCell>
                  <TableCell className="text-right">
                    <div className="flex justify-end gap-2">
                      <Button
                        size="sm"
                        variant="secondary"
                        onClick={() => void toggleRuleSetEnabled(record)}
                        disabled={mutatingRuleSetId === record.id}
                        className={
                          record.enabled
                            ? "border-red-700/70 bg-red-900/60 text-red-200 hover:bg-red-900/80 hover:text-red-100"
                            : "border-emerald-300/70 dark:border-emerald-700/70 bg-emerald-100 dark:bg-emerald-900/60 text-emerald-800 dark:text-emerald-100 hover:bg-emerald-200 dark:hover:bg-emerald-800/80"
                        }
                      >
                        <Power className="mr-1 h-3.5 w-3.5" />
                        {record.enabled ? t("label.disabled") : t("label.enabled")}
                      </Button>
                      <Button
                        size="sm"
                        variant="secondary"
                        onClick={() => editRuleSet(record)}
                      >
                        <Edit className="mr-1 h-3.5 w-3.5" />
                        {t("settings.ruleEdit")}
                      </Button>
                      <Button
                        size="sm"
                        variant="destructive"
                        onClick={() => void deleteRuleSet(record)}
                        disabled={mutatingRuleSetId === record.id}
                      >
                        <Trash2 className="mr-1 h-3.5 w-3.5" />
                        {mutatingRuleSetId === record.id ? t("label.deleting") : t("settings.ruleDelete")}
                      </Button>
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

      <Card>
        <CardHeader>
          <CardTitle className="text-base">
            {editingRuleSetId ? t("settings.ruleUpdate") : t("settings.ruleCreate")}
          </CardTitle>
        </CardHeader>
        <CardContent>
          <form className="space-y-3" onSubmit={submitRuleSet}>
            <div className="grid gap-3 md:grid-cols-3">
              <label>
                <Label className="mb-2 block">{t("settings.ruleName")}</Label>
                <Input
                  value={ruleSetDraft.name}
                  onChange={(e) =>
                    setRuleSetDraft((prev) => ({ ...prev, name: e.target.value }))
                  }
                  required
                  placeholder="my_rule"
                />
              </label>
              <label>
                <Label className="mb-2 block">{t("settings.ruleDescription")}</Label>
                <Input
                  value={ruleSetDraft.description}
                  onChange={(e) =>
                    setRuleSetDraft((prev) => ({ ...prev, description: e.target.value }))
                  }
                  placeholder="Block releases over 100 GiB"
                />
              </label>
              <label>
                <Label className="mb-2 block">{t("settings.rulePriority")}</Label>
                <Input
                  type="number"
                  value={ruleSetDraft.priority}
                  onChange={(e) =>
                    setRuleSetDraft((prev) => ({ ...prev, priority: Number(e.target.value) || 0 }))
                  }
                  placeholder="0"
                />
              </label>
            </div>

            <div>
              <Label className="mb-2 block">{t("settings.ruleRegoSource")}</Label>
              <LazyRegoEditor
                value={ruleSetDraft.regoSource}
                onChange={(value) =>
                  setRuleSetDraft((prev) => ({ ...prev, regoSource: value }))
                }
                height="280px"
              />
            </div>

            <div>
              <Label className="mb-2 block">{t("settings.ruleAppliedFacets")}</Label>
              <p className="mb-2 text-xs text-muted-foreground">{t("settings.ruleAppliedFacetsHelp")}</p>
              <div className="flex items-center gap-4">
                {FACET_OPTIONS.map((opt) => (
                  <label key={opt.value} className="flex items-center gap-2">
                    <input
                      type="checkbox"
                      checked={ruleSetDraft.appliedFacets.includes(opt.value)}
                      onChange={(e) => {
                        setRuleSetDraft((prev) => {
                          const next = e.target.checked
                            ? [...prev.appliedFacets, opt.value]
                            : prev.appliedFacets.filter((f) => f !== opt.value);
                          return { ...prev, appliedFacets: next };
                        });
                      }}
                      className="accent-primary"
                    />
                    <span className="text-sm">{opt.label}</span>
                  </label>
                ))}
              </div>
            </div>

            <label className="flex items-center gap-2">
              <input
                type="checkbox"
                checked={ruleSetDraft.enabled}
                onChange={(e) =>
                  setRuleSetDraft((prev) => ({ ...prev, enabled: e.target.checked }))
                }
                className="accent-primary"
              />
              <span className="text-sm">{t("label.enabled")}</span>
            </label>

            {validationResult ? (
              <div
                className={`rounded border px-3 py-2 text-sm ${
                  validationResult.valid
                    ? "border-emerald-700/50 bg-emerald-900/30 text-emerald-300"
                    : "border-red-700/50 bg-red-900/30 text-red-300"
                }`}
              >
                {validationResult.valid ? (
                  t("settings.ruleValid")
                ) : (
                  <ul className="list-inside list-disc space-y-1">
                    {validationResult.errors.map((err, i) => (
                      <li key={i}>{err}</li>
                    ))}
                  </ul>
                )}
              </div>
            ) : null}

            <div className="flex gap-2">
              <Button type="submit" disabled={mutatingRuleSetId === "new"}>
                {mutatingRuleSetId === "new"
                  ? t("label.saving")
                  : editingRuleSetId
                    ? t("settings.ruleUpdate")
                    : t("settings.ruleCreate")}
              </Button>
              <Button
                type="button"
                variant="secondary"
                onClick={() => void validateDraft()}
                disabled={validating || !ruleSetDraft.regoSource.trim()}
              >
                {validating ? t("settings.ruleValidating") : t("settings.ruleValidate")}
              </Button>
              <Button type="button" variant="secondary" onClick={resetRuleSetDraft}>
                {t("label.cancel")}
              </Button>
            </div>
          </form>
        </CardContent>
      </Card>

      <RulesContextReference t={t} />
    </div>
  );
}
