import * as React from "react";
import { useClient } from "urql";
import { Button } from "@/components/ui/button";
import { TitleAutocompletePicker } from "@/components/common/title-autocomplete-picker";
import { FilterableSelect } from "@/components/ui/filterable-select";
import { useTranslate } from "@/lib/context/translate-context";
import {
  scoringEntryText,
  type ScoringEntryKind,
} from "@/lib/utils/release-decision-explanation";
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from "@/components/ui/collapsible";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { testRuleSetMutation } from "@/lib/graphql/mutations";
import {
  ruleSetTestTitleCollectionsQuery,
  seriesCollectionEpisodesQuery,
  titleMediaFilesQuery,
} from "@/lib/graphql/queries";
import type { RuleSetDraft } from "@/lib/types/rule-sets";
import type { TitleRecord } from "@/lib/types/titles";
import { titleIsEpisodic } from "@/lib/utils/grab-dialog";
import { episodeTitleOrTba } from "@/lib/utils/episode-title";
import {
  canTestRuleSet,
  buildRuleSetTestInput,
  EMPTY_RULE_SET_TEST_LISTING,
  listingInputFromDraft,
  formatSignedScore,
  RuleSetTestRequestController,
  ruleSetTestFingerprint,
  shouldApplyRuleSetTestResponse,
  sizeBytesFromGib,
  storedFileOptions,
  type RuleSetTestListingDraft,
  type RuleSetTestListingFacts,
  type RuleSetTestMode,
  type RuleSetTestSelection,
  type RuleSetTestStoredFileRow,
} from "@/lib/utils/rule-set-test-preview";

type Collection = {
  id: string;
  label?: string | null;
  collectionIndex?: string | number | null;
  collectionType?: string | null;
};
type Episode = {
  id: string;
  seasonNumber?: number | null;
  episodeNumber?: number | null;
  episodeLabel?: string | null;
  title?: string | null;
};
type PreviewEntry = {
  code: string;
  delta: number;
  blocked: boolean;
  kind?: ScoringEntryKind;
};
type PreviewRuleSet = {
  ruleSetId?: string | null;
  ruleSetName?: string;
  origin?: string;
  score?: number;
  matched?: boolean;
  blocked?: boolean;
  isDraft?: boolean;
  messages?: string[];
  entries?: PreviewEntry[];
};
type PreviewResult = {
  score?: number;
  releaseName?: string;
  mediaFileId?: string | null;
  listing?: RuleSetTestListingFacts | null;
  allowed?: boolean;
  blocked?: boolean;
  minimumScoreMet?: boolean;
  profileName?: string | null;
  context?: {
    titleName?: string;
    libraryName?: string;
    facet?: string;
    language?: string | null;
    tags?: string[];
    episodeLabel?: string | null;
  };
  parsed?: Record<string, unknown> | null;
  ruleSets?: PreviewRuleSet[];
  draftContribution?: {
    score?: number;
    matched?: boolean;
    blocked?: boolean;
    applies?: boolean;
    enabled?: boolean;
    message?: string | null;
  } | null;
  errors?: Array<{ code?: string; message: string; ruleSetId?: string | null }>;
};

const PARSED_FIELDS = [
  ["releaseGroup", "Release group"],
  ["quality", "Quality"],
  ["source", "Source"],
  ["season", "Season"],
  ["episode", "Episode"],
  ["edition", "Edition"],
  ["videoCodec", "Video codec"],
  ["audio", "Audio"],
  ["year", "Year"],
  ["audioLanguages", "Audio languages"],
] as const;

function episodeLabel(episode: Episode, t: (key: string) => string): string {
  const prefix = `S${String(episode.seasonNumber ?? 0).padStart(2, "0")}E${String(
    episode.episodeNumber ?? 0,
  ).padStart(2, "0")}`;
  const title = episode.title?.trim() || episode.episodeLabel?.trim();
  return title?.startsWith(prefix) ? title : `${prefix} - ${episodeTitleOrTba(title, t)}`;
}

const LISTING_TEXT_FIELDS = [
  ["publishedAt", "settings.ruleTestListingPublishedAt"],
  ["thumbsUp", "settings.ruleTestListingThumbsUp"],
  ["thumbsDown", "settings.ruleTestListingThumbsDown"],
  ["indexerLanguages", "settings.ruleTestListingIndexerLanguages"],
] as const;

function listingFactLines(
  listing: RuleSetTestListingFacts,
  storedFile: boolean,
  t: ReturnType<typeof useTranslate>,
): Array<readonly [string, string]> {
  const unknown = t("settings.ruleTestListingUnknown");
  const extra = listing.extra ?? {};
  const age =
    listing.ageDays == null
      ? unknown
      : storedFile
        ? t("settings.ruleTestResultAgeAtGrab", { days: listing.ageDays })
        : String(listing.ageDays);
  return [
    [t("settings.ruleTestListingPublishedAt"), listing.publishedAt || unknown],
    [t("settings.ruleTestResultAgeDays"), age],
    [
      t("settings.ruleTestListingThumbsUp"),
      listing.thumbsUp == null ? unknown : String(listing.thumbsUp),
    ],
    [
      t("settings.ruleTestListingThumbsDown"),
      listing.thumbsDown == null ? unknown : String(listing.thumbsDown),
    ],
    [
      t("settings.ruleTestListingPasswordProtected"),
      listing.isPasswordProtected == null
        ? unknown
        : t(
            listing.isPasswordProtected
              ? "settings.ruleTestListingYes"
              : "settings.ruleTestListingNo",
          ),
    ],
    [
      t("settings.ruleTestListingIndexerLanguages"),
      listing.indexerLanguages?.length
        ? listing.indexerLanguages.join(", ")
        : unknown,
    ],
    [
      t("settings.ruleTestListingExtra"),
      Object.keys(extra).length ? JSON.stringify(extra) : unknown,
    ],
    [t("settings.ruleTestResultCapturedAt"), listing.capturedAt || unknown],
  ];
}

function previewEntryLabel(
  entry: PreviewEntry,
  parsed: PreviewResult["parsed"],
): string {
  if (entry.code === "group_unknown") return "Unknown release group";
  if (
    entry.code === "video_codec_quality_high" ||
    entry.code === "video_codec_quality_mid"
  ) {
    const codec = parsed?.videoCodec;
    return `${typeof codec === "string" && codec ? codec : "Video"} codec ${entry.delta > 0 ? "bonus" : "score"}`;
  }
  const label = entry.code.replaceAll("_", " ");
  return label.charAt(0).toUpperCase() + label.slice(1);
}

function parsedLines(parsed: Record<string, unknown> | null | undefined) {
  return PARSED_FIELDS.flatMap(([key, label]) => {
    const value = parsed?.[key];
    return value == null ||
      value === "" ||
      (Array.isArray(value) && value.length === 0)
      ? []
      : [
          [
            label,
            Array.isArray(value) ? value.join(", ") : String(value),
          ] as const,
        ];
  });
}

export function RuleSetTestPanel({
  draft,
  editRuleSetId,
  copySourceRuleSetId,
  open,
  onOpenChange,
  testRuleSetId = null,
}: {
  draft: RuleSetDraft | null;
  editRuleSetId: string | null;
  copySourceRuleSetId: string | null;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  testRuleSetId?: string | null;
}) {
  const client = useClient();
  const t = useTranslate();
  const [selectedTitle, setSelectedTitle] = React.useState<TitleRecord | null>(
    null,
  );
  const [episodes, setEpisodes] = React.useState<Episode[]>([]);
  const [episodeId, setEpisodeId] = React.useState("");
  const [releaseName, setReleaseName] = React.useState("");
  const [sizeGib, setSizeGib] = React.useState("");
  const [mode, setMode] = React.useState<RuleSetTestMode>("release");
  const [storedFiles, setStoredFiles] = React.useState<RuleSetTestStoredFileRow[]>([]);
  const [mediaFileId, setMediaFileId] = React.useState("");
  const [loadingFiles, setLoadingFiles] = React.useState(false);
  const [listingOpen, setListingOpen] = React.useState(false);
  const [listing, setListing] = React.useState<RuleSetTestListingDraft>(
    EMPTY_RULE_SET_TEST_LISTING,
  );
  const [loadingEpisodes, setLoadingEpisodes] = React.useState(false);
  const [testing, setTesting] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const [result, setResult] = React.useState<PreviewResult | null>(null);
  const [resultFingerprint, setResultFingerprint] = React.useState<
    string | null
  >(null);
  const controllerRef = React.useRef(new RuleSetTestRequestController());
  const committedFingerprintRef = React.useRef("");

  const storedFileMode = mode === "storedFile";
  const fileOptions = React.useMemo(
    () => storedFileOptions(storedFiles, episodeId || null),
    [storedFiles, episodeId],
  );
  // A file that does not cover the selected episode is never offered, and
  // never stays selected.
  const selectedFileId = fileOptions.some((file) => file.id === mediaFileId)
    ? mediaFileId
    : "";
  const selection: RuleSetTestSelection = {
    titleId: selectedTitle?.id ?? null,
    episodeId: episodeId || null,
    releaseName,
    sizeGib,
    mode,
    mediaFileId: selectedFileId || null,
    listing,
  };
  const episodic = titleIsEpisodic(selectedTitle);
  const savedRuleSetMode = testRuleSetId !== null;
  const fingerprint = ruleSetTestFingerprint(
    draft,
    selection,
    editRuleSetId,
    copySourceRuleSetId,
    testRuleSetId,
  );
  const stale = result !== null && resultFingerprint !== fingerprint;
  const canTest = canTestRuleSet(selection, episodic) && !testing;
  const incomplete = Boolean(result?.errors?.length);

  React.useLayoutEffect(() => {
    committedFingerprintRef.current = fingerprint;
  }, [fingerprint]);
  React.useEffect(() => {
    const controller = controllerRef.current;
    controller.activate();
    return () => controller.dispose();
  }, []);

  React.useEffect(() => {
    if (!selectedTitle || !episodic) {
      setEpisodes([]);
      setEpisodeId("");
      setLoadingEpisodes(false);
      return;
    }
    let cancelled = false;
    void (async () => {
      setLoadingEpisodes(true);
      setEpisodeId("");
      try {
        const { data, error: collectionsError } = await client
          .query(ruleSetTestTitleCollectionsQuery, { id: selectedTitle.id })
          .toPromise();
        if (collectionsError) throw collectionsError;
        const collections = (data?.title?.collections ?? []) as Collection[];
        const episodeGroups = await Promise.all(
          collections.map(async (collection) => {
            const { data: episodesData, error: episodesError } = await client
              .query(seriesCollectionEpisodesQuery, { id: collection.id })
              .toPromise();
            if (episodesError) throw episodesError;
            return (episodesData?.collectionById?.episodes ?? []) as Episode[];
          }),
        );
        if (!cancelled) {
          setEpisodes(
            episodeGroups
              .flat()
              .sort(
                (left, right) =>
                  (left.seasonNumber ?? 0) - (right.seasonNumber ?? 0) ||
                  (left.episodeNumber ?? 0) - (right.episodeNumber ?? 0),
              ),
          );
        }
      } catch (loadError) {
        if (!cancelled)
          setError(
            loadError instanceof Error
              ? loadError.message
              : "Unable to load episodes.",
          );
      } finally {
        if (!cancelled) setLoadingEpisodes(false);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [client, episodic, selectedTitle]);

  React.useEffect(() => {
    if (!selectedTitle || mode !== "storedFile") {
      setStoredFiles([]);
      setMediaFileId("");
      setLoadingFiles(false);
      return;
    }
    let cancelled = false;
    void (async () => {
      setLoadingFiles(true);
      setMediaFileId("");
      try {
        const { data, error: filesError } = await client
          .query(titleMediaFilesQuery, { id: selectedTitle.id })
          .toPromise();
        if (filesError) throw filesError;
        if (!cancelled)
          setStoredFiles((data?.title?.mediaFiles ?? []) as RuleSetTestStoredFileRow[]);
      } catch {
        if (!cancelled) setError(t("settings.ruleTestStoredFileLoadFailed"));
      } finally {
        if (!cancelled) setLoadingFiles(false);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [client, mode, selectedTitle, t]);

  const updateListing = (field: keyof RuleSetTestListingDraft, value: string) =>
    setListing((current) => ({ ...current, [field]: value }));

  const clearEpisodeChoices = () => {
    setEpisodes([]);
    setEpisodeId("");
  };
  const handleSelectedTitleChange = (title: TitleRecord | null) => {
    setSelectedTitle(title);
    clearEpisodeChoices();
    setLoadingEpisodes(Boolean(title && titleIsEpisodic(title)));
    setError(null);
  };

  const test = async () => {
    if (!canTest || !selectedTitle) return;
    const size = storedFileMode ? { value: undefined } : sizeBytesFromGib(sizeGib);
    if ("error" in size) {
      setError(size.error);
      return;
    }
    const listingInput = storedFileMode
      ? { value: undefined }
      : listingInputFromDraft(listing);
    if ("error" in listingInput) {
      setError(t(listingInput.error));
      return;
    }
    const request = controllerRef.current.begin();
    if (request == null) return;
    setTesting(true);
    setError(null);
    try {
      const input = buildRuleSetTestInput({
        draft,
        editRuleSetId,
        copySourceRuleSetId,
        testRuleSetId,
        titleId: selectedTitle.id,
        episodeId: episodic ? episodeId || undefined : undefined,
        ...(storedFileMode
          ? { mediaFileId: selectedFileId }
          : {
              releaseName: releaseName.trim(),
              sizeBytes: size.value,
              listing: listingInput.value,
            }),
      });
      const { data, error: mutationError } = await client
        .mutation(testRuleSetMutation, { input })
        .toPromise();
      if (mutationError) throw mutationError;
      if (
        !shouldApplyRuleSetTestResponse(
          request,
          controllerRef.current.isCurrent(request) ? request : -1,
          fingerprint,
          committedFingerprintRef.current,
        )
      )
        return;
      setResult((data?.testRuleSet ?? null) as PreviewResult | null);
      setResultFingerprint(fingerprint);
    } catch (testError) {
      if (
        shouldApplyRuleSetTestResponse(
          request,
          controllerRef.current.isCurrent(request) ? request : -1,
          fingerprint,
          committedFingerprintRef.current,
        )
      )
        setError(
          testError instanceof Error
            ? testError.message
            : "Scoring preview failed.",
        );
    } finally {
      if (controllerRef.current.finish(request)) setTesting(false);
    }
  };

  const groupedRuleSets = React.useMemo(() => {
    const groups = new Map<string, PreviewRuleSet[]>();
    for (const item of result?.ruleSets ?? []) {
      const origin = item.origin || "Rules";
      groups.set(origin, [...(groups.get(origin) ?? []), item]);
    }
    return [...groups.entries()];
  }, [result]);

  return (
    <Collapsible
      open={open}
      onOpenChange={onOpenChange}
      className="rounded border border-border"
    >
      <CollapsibleContent className="px-3 py-3">
        <p className="mb-3 text-xs text-muted-foreground">
          Scoring preview only. This does not save{" "}
          {savedRuleSetMode ? "this rule" : "this draft"}, admit a download, or
          create history.
        </p>
        <div className="space-y-3">
          <ToggleGroup
            type="single"
            variant="outline"
            size="sm"
            value={mode}
            onValueChange={(value) => {
              if (value === "release" || value === "storedFile") {
                setMode(value);
                setError(null);
              }
            }}
            aria-label={t("settings.ruleTestModeLabel")}
          >
            <ToggleGroupItem value="release" variant="outline" size="sm">
              {t("settings.ruleTestModeRelease")}
            </ToggleGroupItem>
            <ToggleGroupItem value="storedFile" variant="outline" size="sm">
              {t("settings.ruleTestModeStoredFile")}
            </ToggleGroupItem>
          </ToggleGroup>
          {storedFileMode ? null : (
            <div>
              <Label
                htmlFor="settings-rule-test-release"
                className="mb-1 block"
              >
                Release name
              </Label>
              <Input
                id="settings-rule-test-release"
                value={releaseName}
                onChange={(event) => setReleaseName(event.target.value)}
                placeholder="Example.Show.S01E01.1080p.WEB-DL"
              />
            </div>
          )}
          <div className="grid gap-3 md:grid-cols-2">
            <div>
              <Label className="mb-1 block">Library title</Label>
              <TitleAutocompletePicker
                selectedTitle={selectedTitle}
                selectedTitleId={selectedTitle?.id ?? null}
                onSelectedTitleChange={handleSelectedTitleChange}
                placeholder="Search movies, shows, and anime"
                ariaLabel="Library title"
              />
            </div>
            {episodic ? (
              <div>
                <Label
                  htmlFor="settings-rule-test-episode"
                  className="mb-1 block"
                >
                  Episode
                </Label>
                <FilterableSelect
                  id="settings-rule-test-episode"
                  value={episodeId}
                  onValueChange={setEpisodeId}
                  options={episodes.map((episode) => ({
                    value: episode.id,
                    label: episodeLabel(episode, t),
                  }))}
                  placeholder={
                    loadingEpisodes ? "Loading episodes…" : "Select an episode"
                  }
                  filterPlaceholder="Filter episodes"
                  filterByValue={false}
                  ariaLabel="Episode"
                  optionIdPrefix="settings-rule-test-episode-option"
                  disabled={loadingEpisodes}
                />
              </div>
            ) : null}
          </div>
          {storedFileMode ? (
            <div>
              <Label
                htmlFor="settings-rule-test-stored-file"
                className="mb-1 block"
              >
                {t("settings.ruleTestStoredFile")}
              </Label>
              <FilterableSelect
                id="settings-rule-test-stored-file"
                value={selectedFileId}
                onValueChange={setMediaFileId}
                options={fileOptions.map((file) => ({
                  value: file.id,
                  label: file.label,
                }))}
                placeholder={
                  !selectedTitle
                    ? t("settings.ruleTestStoredFileSelectTitle")
                    : loadingFiles
                      ? t("settings.ruleTestStoredFileLoading")
                      : fileOptions.length
                        ? t("settings.ruleTestStoredFilePlaceholder")
                        : t("settings.ruleTestStoredFileNone")
                }
                filterPlaceholder={t("settings.ruleTestStoredFileFilter")}
                filterByValue={false}
                ariaLabel={t("settings.ruleTestStoredFile")}
                optionIdPrefix="settings-rule-test-stored-file-option"
                disabled={!selectedTitle || loadingFiles || !fileOptions.length}
              />
              <p className="mt-1 text-xs text-muted-foreground">
                {t("settings.ruleTestStoredFileHelp")}
              </p>
            </div>
          ) : (
            <>
              <div className="max-w-sm">
                <Label
                  htmlFor="settings-rule-test-size"
                  className="mb-1 block"
                >
                  Size (GiB, optional)
                </Label>
                <Input
                  id="settings-rule-test-size"
                  inputMode="decimal"
                  value={sizeGib}
                  onChange={(event) => setSizeGib(event.target.value)}
                  placeholder="Unknown"
                />
              </div>
              <Collapsible
                open={listingOpen}
                onOpenChange={setListingOpen}
                className="rounded border border-border"
              >
                <CollapsibleTrigger asChild>
                  <button
                    type="button"
                    className="flex w-full items-center justify-between px-3 py-2 text-left text-sm font-medium"
                  >
                    {t("settings.ruleTestListingFacts")}
                    <span aria-hidden="true" className="text-muted-foreground">
                      {listingOpen ? "−" : "+"}
                    </span>
                  </button>
                </CollapsibleTrigger>
                <CollapsibleContent className="space-y-3 border-t border-border px-3 py-3">
                  <p className="text-xs text-muted-foreground">
                    {t("settings.ruleTestListingFactsHelp")}
                  </p>
                  <div className="grid gap-3 md:grid-cols-2">
                    {LISTING_TEXT_FIELDS.map(([field, labelKey]) => (
                      <div key={field}>
                        <Label
                          htmlFor={`settings-rule-test-listing-${field}`}
                          className="mb-1 block"
                        >
                          {t(labelKey)}
                        </Label>
                        <Input
                          id={`settings-rule-test-listing-${field}`}
                          inputMode={
                            field === "thumbsUp" || field === "thumbsDown"
                              ? "numeric"
                              : undefined
                          }
                          value={listing[field]}
                          onChange={(event) =>
                            updateListing(field, event.target.value)
                          }
                          placeholder={
                            field === "publishedAt"
                              ? t(
                                  "settings.ruleTestListingPublishedAtPlaceholder",
                                )
                              : field === "indexerLanguages"
                                ? "en, de"
                                : t("settings.ruleTestListingUnknown")
                          }
                        />
                      </div>
                    ))}
                    <div>
                      <p
                        id="settings-rule-test-listing-password-protected"
                        className="mb-1 block text-sm leading-none font-medium select-none"
                      >
                        {t("settings.ruleTestListingPasswordProtected")}
                      </p>
                      <ToggleGroup
                        type="single"
                        variant="outline"
                        size="sm"
                        value={listing.isPasswordProtected || "unknown"}
                        onValueChange={(value) =>
                          updateListing(
                            "isPasswordProtected",
                            value === "true" || value === "false" ? value : "",
                          )
                        }
                        aria-labelledby="settings-rule-test-listing-password-protected"
                      >
                        <ToggleGroupItem value="unknown" variant="outline" size="sm">
                          {t("settings.ruleTestListingUnknown")}
                        </ToggleGroupItem>
                        <ToggleGroupItem value="true" variant="outline" size="sm">
                          {t("settings.ruleTestListingYes")}
                        </ToggleGroupItem>
                        <ToggleGroupItem value="false" variant="outline" size="sm">
                          {t("settings.ruleTestListingNo")}
                        </ToggleGroupItem>
                      </ToggleGroup>
                    </div>
                  </div>
                  <div>
                    <Label
                      htmlFor="settings-rule-test-listing-extra"
                      className="mb-1 block"
                    >
                      {t("settings.ruleTestListingExtra")}
                    </Label>
                    <textarea
                      id="settings-rule-test-listing-extra"
                      className="min-h-16 w-full rounded-md border border-input bg-transparent px-3 py-2 font-mono text-xs"
                      value={listing.extra}
                      onChange={(event) =>
                        updateListing("extra", event.target.value)
                      }
                      placeholder='{"freeleech": true}'
                      spellCheck={false}
                    />
                  </div>
                </CollapsibleContent>
              </Collapsible>
            </>
          )}
        </div>
        <div className="mt-3 flex items-center gap-3">
          <Button type="button" onClick={() => void test()} disabled={!canTest}>
            {testing ? "Testing…" : "Run test"}
          </Button>
          {stale ? (
            <span className="text-xs text-[var(--scry-warning-text)]">
              Inputs changed; result is stale.
            </span>
          ) : null}
        </div>
        {error ? (
          <div className="mt-3 rounded border border-[var(--scry-danger-border)] bg-[var(--scry-danger-bg)] px-3 py-2 text-sm text-[var(--scry-danger-text)]">
            {error}
          </div>
        ) : null}
        {result ? (
          <div
            className="mt-3 space-y-3 rounded border border-border p-3"
            aria-live="polite"
          >
            <div className="grid items-start gap-4 lg:grid-cols-[auto_minmax(0,1fr)] lg:gap-6">
              <div className="space-y-3">
                <div className="grid grid-cols-2 gap-3">
                  {[
                    {
                      label: "This rule’s contribution",
                      score: result.draftContribution?.score ?? 0,
                    },
                    {
                      label: "Overall custom score",
                      score: result.score ?? 0,
                    },
                  ].map(({ label, score }) => (
                    <div
                      key={label}
                      className={
                        score > 0
                          ? "rounded border border-[var(--scry-success-border)] bg-[var(--scry-success-bg)] px-4 py-3 text-[var(--scry-success-text)]"
                          : score < 0
                            ? "rounded border border-[var(--scry-danger-border)] bg-[var(--scry-danger-bg)] px-4 py-3 text-[var(--scry-danger-text)]"
                            : "rounded border border-border bg-muted/50 px-4 py-3"
                      }
                    >
                      <p className="text-xs font-medium uppercase tracking-wide">
                        {label}
                      </p>
                      <p className="text-3xl font-semibold tabular-nums">
                        {formatSignedScore(score)}
                      </p>
                    </div>
                  ))}
                </div>
                <div>
                  <p
                    className={
                      incomplete
                        ? "font-medium text-[var(--scry-warning-text)]"
                        : result.allowed
                          ? "font-medium text-[var(--scry-success-text)]"
                          : "font-medium text-[var(--scry-danger-text)]"
                    }
                  >
                    {incomplete
                      ? "Incomplete"
                      : result.allowed
                        ? "Allowed"
                        : "Not allowed"}
                  </p>
                  <p className="text-xs text-muted-foreground">
                    {incomplete
                      ? "Evaluation completed with errors."
                      : "Scoring policy"}
                    {result.minimumScoreMet === false
                      ? " · Minimum score not met"
                      : ""}
                  </p>
                </div>
                {result.draftContribution ? (
                  <div className="text-xs">
                    <p className="text-muted-foreground">
                      {result.draftContribution.enabled === false
                        ? "This rule is disabled."
                        : result.draftContribution.applies === false
                          ? "Does not apply to this media type."
                          : result.draftContribution.message ||
                            (result.draftContribution.matched
                              ? "This rule matched the release."
                              : "This rule did not match the release.")}
                    </p>
                  </div>
                ) : null}
              </div>
              <div className="min-w-0 space-y-3 lg:border-l lg:border-border lg:pl-6">
                {groupedRuleSets.map(([origin, entries]) => (
                  <div key={origin} className="text-xs">
                    {origin !== "user" ? (
                      <p className="mb-1 font-medium">{origin}</p>
                    ) : null}
                    <ul className="space-y-2">
                      {entries.map((entry, index) => (
                        <li
                          key={`${entry.ruleSetId || entry.ruleSetName || index}`}
                        >
                          <div className="flex justify-between gap-3">
                            <span>
                              {entry.ruleSetName === "TRaSH Guides source video"
                                ? "Source & video codec"
                                : entry.ruleSetName || "Unnamed rule"}
                              {entry.isDraft ||
                              (savedRuleSetMode &&
                                entry.ruleSetId === testRuleSetId)
                                ? " (this rule)"
                                : ""}
                              {entry.messages?.length
                                ? ` — ${entry.messages.join("; ")}`
                                : ""}
                            </span>
                            <span className="shrink-0 tabular-nums">
                              {entry.matched
                                ? formatSignedScore(entry.score ?? 0)
                                : "No match"}
                            </span>
                          </div>
                          {entry.entries?.length ? (
                            <ul className="mt-1 space-y-1 border-l border-border pl-2 text-muted-foreground">
                              {entry.entries.map((scoreEntry, entryIndex) => (
                                <li
                                  key={`${scoreEntry.code}-${entryIndex}`}
                                  className="flex justify-between gap-3"
                                >
                                  <span className="min-w-0 break-words">
                                    {previewEntryLabel(scoreEntry, result.parsed)}
                                  </span>
                                  <span className="shrink-0 tabular-nums">
                                    {scoringEntryText(scoreEntry, t)}
                                  </span>
                                </li>
                              ))}
                            </ul>
                          ) : null}
                        </li>
                      ))}
                    </ul>
                  </div>
                ))}
              </div>
            </div>
            {incomplete ? (
              <div className="rounded border border-[var(--scry-warning-border)] bg-[var(--scry-warning-bg)] px-3 py-2 text-sm text-[var(--scry-warning-text)]">
                <p className="font-medium">Incomplete scoring preview</p>
                <p>
                  This result includes evaluation errors, so it cannot determine
                  download admission.
                </p>
              </div>
            ) : null}
            <div className="space-y-1 text-xs">
              <p>
                <span className="font-medium">Profile:</span>{" "}
                {result.profileName || "Unavailable"}
              </p>
              {result.context ? (
                <p>
                  <span className="font-medium">Context:</span>{" "}
                  {[
                    result.context.titleName,
                    result.context.libraryName,
                    result.context.facet,
                    result.context.language,
                    result.context.episodeLabel,
                  ]
                    .filter(Boolean)
                    .join(" · ") || "Unavailable"}
                </p>
              ) : null}
              {result.context?.tags?.length ? (
                <p>
                  <span className="font-medium">Tags:</span>{" "}
                  {result.context.tags.join(", ")}
                </p>
              ) : null}
            </div>
            <div className="space-y-1 text-xs">
              {result.releaseName ? (
                <p className="break-words">
                  <span className="font-medium">
                    {t("settings.ruleTestResultRelease")}:
                  </span>{" "}
                  {result.releaseName}
                </p>
              ) : null}
              {result.listing ? (
                <div>
                  <p className="font-medium">
                    {t("settings.ruleTestListingFacts")}
                  </p>
                  <dl className="grid grid-cols-[max-content_minmax(0,1fr)] gap-x-3 gap-y-1">
                    {listingFactLines(
                      result.listing,
                      Boolean(result.mediaFileId),
                      t,
                    ).map(([label, value]) => (
                      <React.Fragment key={label}>
                        <dt className="text-muted-foreground">{label}</dt>
                        <dd className="break-words">{value}</dd>
                      </React.Fragment>
                    ))}
                  </dl>
                </div>
              ) : result.mediaFileId ? (
                <p className="text-[var(--scry-warning-text)]">
                  {t("settings.ruleTestResultListingUnknown")}
                </p>
              ) : null}
            </div>
            <div className="text-xs">
              <dl className="grid grid-cols-[max-content_minmax(0,1fr)] gap-x-3 gap-y-1">
                {parsedLines(result.parsed).map(([label, value]) => (
                  <React.Fragment key={label}>
                    <dt className="text-muted-foreground">{label}</dt>
                    <dd className="break-words">{value}</dd>
                  </React.Fragment>
                ))}
                <dt className="text-muted-foreground">Size</dt>
                <dd>
                  {result.parsed?.sizeBytes == null
                    ? "Unknown"
                    : `${result.parsed.sizeBytes} bytes`}
                </dd>
              </dl>
            </div>
            {result.errors?.length ? (
              <div className="rounded border border-[var(--scry-danger-border)] bg-[var(--scry-danger-bg)] px-2 py-2 text-xs text-[var(--scry-danger-text)]">
                <p className="font-medium">Evaluation errors</p>
                <ul className="list-disc pl-4">
                  {result.errors.map((item, index) => (
                    <li key={`${item.code || "error"}-${index}`}>
                      {item.message}
                    </li>
                  ))}
                </ul>
              </div>
            ) : null}
          </div>
        ) : null}
      </CollapsibleContent>
    </Collapsible>
  );
}
