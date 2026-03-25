import * as React from "react";
import { Edit, Trash2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Checkbox } from "@/components/ui/checkbox";
import { Input, integerInputProps, sanitizeDigits } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
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
  DelayProfileDraft,
  DelayProfileFacet,
  ParsedDelayProfile,
} from "@/lib/types/delay-profiles";
import { FACET_OPTIONS } from "@/lib/utils/delay-profiles";

type SettingsDelayProfilesSectionProps = {
  loading: boolean;
  saving: boolean;
  profiles: ParsedDelayProfile[];
  parseError: string;
  draft: DelayProfileDraft;
  setDraft: React.Dispatch<React.SetStateAction<DelayProfileDraft>>;
  saveProfile: (event?: React.FormEvent<HTMLFormElement>) => void;
  deleteProfile: (profileId: string) => void;
  loadProfileById: (profileId: string) => void;
  resetDraft: () => void;
};

const FACET_LABELS: Record<string, string> = {
  movie: "Movies",
  series: "TV Series",
  anime: "Anime",
};

export function SettingsDelayProfilesSection({
  loading,
  saving,
  profiles,
  parseError,
  draft,
  setDraft,
  saveProfile,
  deleteProfile,
  loadProfileById,
  resetDraft,
}: SettingsDelayProfilesSectionProps) {
  const t = useTranslate();

  const isEditing = !!draft.id;

  function updateField<K extends keyof DelayProfileDraft>(field: K, value: DelayProfileDraft[K]) {
    setDraft((prev) => ({ ...prev, [field]: value }));
  }

  function parseIntegerInput(raw: string) {
    const nextValue = sanitizeDigits(raw);
    return nextValue === "" ? 0 : Number(nextValue);
  }

  function toggleFacet(facet: DelayProfileFacet) {
    setDraft((prev) => {
      const has = prev.applies_to_facets.includes(facet);
      return {
        ...prev,
        applies_to_facets: has
          ? prev.applies_to_facets.filter((f) => f !== facet)
          : [...prev.applies_to_facets, facet],
      };
    });
  }

  return (
    <div className="space-y-6">
      {parseError && (
        <div className="rounded border border-rose-500/30 bg-rose-500/10 p-3 text-sm text-rose-300">
          {parseError}
        </div>
      )}

      {/* Existing profiles table */}
      <Card className="bg-card border-border">
        <CardHeader>
          <CardTitle className="text-base">{t("settings.delayProfileExisting")}</CardTitle>
        </CardHeader>
        <CardContent>
          {loading ? (
            <p className="text-muted-foreground text-sm">{t("label.loading")}</p>
          ) : profiles.length === 0 ? (
            <p className="text-muted-foreground text-sm">{t("settings.delayProfileNone")}</p>
          ) : (
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>{t("settings.delayProfileNameLabel")}</TableHead>
                  <TableHead>{t("settings.delayProfileUsenetDelay")}</TableHead>
                  <TableHead>{t("settings.delayProfileTorrentDelay")}</TableHead>
                  <TableHead>{t("settings.delayProfilePreferred")}</TableHead>
                  <TableHead>{t("settings.delayProfileMinAge")}</TableHead>
                  <TableHead>{t("settings.delayProfileBypassLabel")}</TableHead>
                  <TableHead>{t("settings.delayProfileFacetsLabel")}</TableHead>
                  <TableHead>{t("settings.delayProfilePriorityLabel")}</TableHead>
                  <TableHead>{t("settings.delayProfileEnabledLabel")}</TableHead>
                  <TableHead className="w-24" />
                </TableRow>
              </TableHeader>
              <TableBody>
                {profiles.map((profile) => (
                  <TableRow key={profile.id}>
                    <TableCell className="font-medium">{profile.name}</TableCell>
                    <TableCell>{profile.usenet_delay_minutes}m</TableCell>
                    <TableCell>{profile.torrent_delay_minutes}m</TableCell>
                    <TableCell>{profile.preferred_protocol}</TableCell>
                    <TableCell>{profile.min_age_minutes > 0 ? `${profile.min_age_minutes}m` : "—"}</TableCell>
                    <TableCell>
                      {profile.bypass_score_threshold != null
                        ? `≥ ${profile.bypass_score_threshold}`
                        : "—"}
                    </TableCell>
                    <TableCell>
                      {profile.applies_to_facets.length === 0
                        ? t("settings.delayProfileAllFacets")
                        : profile.applies_to_facets
                            .map((f) => FACET_LABELS[f] ?? f)
                            .join(", ")}
                    </TableCell>
                    <TableCell>{profile.priority}</TableCell>
                    <TableCell>{profile.enabled ? "✓" : "✗"}</TableCell>
                    <TableCell>
                      <div className="flex gap-1">
                        <Button
                          variant="ghost"
                          size="icon"
                          onClick={() => loadProfileById(profile.id)}
                          title={t("label.load")}
                        >
                          <Edit className="h-4 w-4" />
                        </Button>
                        <Button
                          variant="ghost"
                          size="icon"
                          onClick={() => deleteProfile(profile.id)}
                          disabled={saving}
                          title={t("label.delete")}
                        >
                          <Trash2 className="h-4 w-4 text-rose-400" />
                        </Button>
                      </div>
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          )}
        </CardContent>
      </Card>

      {/* Draft editor */}
      <Card className="bg-card border-border">
        <CardHeader>
          <CardTitle className="text-base">
            {isEditing
              ? t("settings.delayProfileEdit")
              : t("settings.delayProfileCreate")}
          </CardTitle>
        </CardHeader>
        <CardContent>
          <form onSubmit={saveProfile} className="space-y-4">
            {/* Name */}
            <div className="space-y-1.5">
              <Label htmlFor="dp-name">{t("settings.delayProfileNameLabel")}</Label>
              <Input
                id="dp-name"
                value={draft.name}
                onChange={(e) => updateField("name", e.target.value)}
                placeholder={t("settings.delayProfileNamePlaceholder")}
              />
            </div>

            {/* Protocol delays */}
            <div className="grid grid-cols-2 gap-4">
              <div className="space-y-1.5">
                <Label htmlFor="dp-usenet-delay">{t("settings.delayProfileUsenetDelay")}</Label>
                <Input
                  id="dp-usenet-delay"
                  {...integerInputProps}
                  value={draft.usenet_delay_minutes}
                  onChange={(e) => updateField("usenet_delay_minutes", parseIntegerInput(e.target.value))}
                />
                <p className="text-muted-foreground text-xs">
                  {t("settings.delayProfileUsenetDelayHelp")}
                </p>
              </div>
              <div className="space-y-1.5">
                <Label htmlFor="dp-torrent-delay">{t("settings.delayProfileTorrentDelay")}</Label>
                <Input
                  id="dp-torrent-delay"
                  {...integerInputProps}
                  value={draft.torrent_delay_minutes}
                  onChange={(e) => updateField("torrent_delay_minutes", parseIntegerInput(e.target.value))}
                />
                <p className="text-muted-foreground text-xs">
                  {t("settings.delayProfileTorrentDelayHelp")}
                </p>
              </div>
            </div>

            {/* Preferred protocol */}
            <div className="space-y-1.5">
              <Label>{t("settings.delayProfilePreferred")}</Label>
              <div className="flex gap-4">
                {(["usenet", "torrent"] as const).map((proto) => (
                  <label key={proto} className="flex items-center gap-2 text-sm">
                    <input
                      type="radio"
                      name="preferred_protocol"
                      value={proto}
                      checked={draft.preferred_protocol === proto}
                      onChange={() => updateField("preferred_protocol", proto)}
                      className="accent-primary"
                    />
                    {proto === "usenet" ? "Usenet" : "Torrent"}
                  </label>
                ))}
              </div>
              <p className="text-muted-foreground text-xs">
                {t("settings.delayProfilePreferredHelp")}
              </p>
            </div>

            {/* Minimum age (usenet) */}
            <div className="space-y-1.5">
              <Label htmlFor="dp-min-age">{t("settings.delayProfileMinAge")}</Label>
              <Input
                id="dp-min-age"
                {...integerInputProps}
                value={draft.min_age_minutes}
                onChange={(e) => updateField("min_age_minutes", parseIntegerInput(e.target.value))}
              />
              <p className="text-muted-foreground text-xs">
                {t("settings.delayProfileMinAgeHelp")}
              </p>
            </div>

            {/* Bypass score threshold */}
            <div className="space-y-1.5">
              <Label htmlFor="dp-bypass">{t("settings.delayProfileBypassLabel")}</Label>
              <Input
                id="dp-bypass"
                {...integerInputProps}
                value={draft.bypass_score_threshold ?? ""}
                onChange={(e) => {
                  const val = sanitizeDigits(e.target.value);
                  updateField(
                    "bypass_score_threshold",
                    val === "" ? null : Number(val),
                  );
                }}
                placeholder={t("settings.delayProfileBypassPlaceholder")}
              />
              <p className="text-muted-foreground text-xs">
                {t("settings.delayProfileBypassHelp")}
              </p>
            </div>

            {/* Applies to facets */}
            <div className="space-y-1.5">
              <Label>{t("settings.delayProfileFacetsLabel")}</Label>
              <div className="flex gap-4">
                {FACET_OPTIONS.map((facet) => (
                  <label key={facet} className="flex items-center gap-2 text-sm">
                    <Checkbox
                      checked={draft.applies_to_facets.includes(facet)}
                      onCheckedChange={() => toggleFacet(facet)}
                    />
                    {FACET_LABELS[facet] ?? facet}
                  </label>
                ))}
              </div>
              <p className="text-muted-foreground text-xs">
                {t("settings.delayProfileFacetsHelp")}
              </p>
            </div>

            {/* Tags */}
            <div className="space-y-1.5">
              <Label htmlFor="dp-tags">{t("settings.delayProfileTagsLabel")}</Label>
              <Input
                id="dp-tags"
                value={draft.tags.join(", ")}
                onChange={(e) =>
                  updateField(
                    "tags",
                    e.target.value
                      .split(",")
                      .map((s) => s.trim())
                      .filter(Boolean),
                  )
                }
                placeholder={t("settings.delayProfileTagsPlaceholder")}
              />
              <p className="text-muted-foreground text-xs">
                {t("settings.delayProfileTagsHelp")}
              </p>
            </div>

            {/* Priority */}
            <div className="space-y-1.5">
              <Label htmlFor="dp-priority">{t("settings.delayProfilePriorityLabel")}</Label>
              <Input
                id="dp-priority"
                {...integerInputProps}
                value={draft.priority}
                onChange={(e) => updateField("priority", parseIntegerInput(e.target.value))}
              />
              <p className="text-muted-foreground text-xs">
                {t("settings.delayProfilePriorityHelp")}
              </p>
            </div>

            {/* Enabled */}
            <label className="flex items-center gap-2 text-sm">
              <Checkbox
                checked={draft.enabled}
                onCheckedChange={(checked) =>
                  updateField("enabled", checked === true)
                }
              />
              {t("settings.delayProfileEnabledLabel")}
            </label>

            {/* Actions */}
            <div className="flex gap-2 pt-2">
              <Button type="submit" disabled={saving}>
                {saving
                  ? t("label.saving")
                  : isEditing
                    ? t("label.save")
                    : t("settings.delayProfileCreate")}
              </Button>
              {isEditing && (
                <Button type="button" variant="outline" onClick={resetDraft}>
                  {t("label.cancel")}
                </Button>
              )}
            </div>
          </form>
        </CardContent>
      </Card>
    </div>
  );
}
