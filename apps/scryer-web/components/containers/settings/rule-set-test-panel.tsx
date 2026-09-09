import * as React from "react";
import { useClient } from "urql";
import { Button } from "@/components/ui/button";
import { TitleAutocompletePicker } from "@/components/common/title-autocomplete-picker";
import { useTranslate } from "@/lib/context/translate-context";
import { scoringEntryText, type ScoringEntryKind } from "@/lib/utils/release-decision-explanation";
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from "@/components/ui/collapsible";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { testRuleSetMutation } from "@/lib/graphql/mutations";
import {
  ruleSetTestTitleCollectionsQuery,
  seriesCollectionEpisodesQuery,
} from "@/lib/graphql/queries";
import type { RuleSetDraft } from "@/lib/types/rule-sets";
import type { TitleRecord } from "@/lib/types/titles";
import { titleIsEpisodic } from "@/lib/utils/grab-dialog";
import {
  canTestRuleSet,
  formatSignedScore,
  RuleSetTestRequestController,
  ruleSetTestFingerprint,
  shouldApplyRuleSetTestResponse,
  sizeBytesFromGib,
  type RuleSetTestSelection,
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
type PreviewEntry = { code: string; delta: number; blocked: boolean; kind?: ScoringEntryKind };
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

function collectionLabel(collection: Collection): string {
  return collection.label ||
    (collection.collectionIndex != null
      ? `Season ${collection.collectionIndex}`
      : collection.collectionType || "Episodes");
}

function episodeLabel(episode: Episode): string {
  return episode.episodeLabel ||
    `S${String(episode.seasonNumber ?? 0).padStart(2, "0")}E${String(
      episode.episodeNumber ?? 0,
    ).padStart(2, "0")}${episode.title ? ` — ${episode.title}` : ""}`;
}

function parsedLines(parsed: Record<string, unknown> | null | undefined) {
  return PARSED_FIELDS.flatMap(([key, label]) => {
    const value = parsed?.[key];
    return value == null || value === "" || (Array.isArray(value) && value.length === 0)
      ? []
      : [[label, Array.isArray(value) ? value.join(", ") : String(value)] as const];
  });
}

export function RuleSetTestPanel({
  draft,
  editRuleSetId,
  copySourceRuleSetId,
}: {
  draft: RuleSetDraft;
  editRuleSetId: string | null;
  copySourceRuleSetId: string | null;
}) {
  const client = useClient();
  const t = useTranslate();
  const [open, setOpen] = React.useState(false);
  const [selectedTitle, setSelectedTitle] = React.useState<TitleRecord | null>(null);
  const [collections, setCollections] = React.useState<Collection[]>([]);
  const [collectionId, setCollectionId] = React.useState("");
  const [episodes, setEpisodes] = React.useState<Episode[]>([]);
  const [episodeId, setEpisodeId] = React.useState("");
  const [releaseName, setReleaseName] = React.useState("");
  const [sizeGib, setSizeGib] = React.useState("");
  const [loadingEpisodes, setLoadingEpisodes] = React.useState(false);
  const [testing, setTesting] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const [result, setResult] = React.useState<PreviewResult | null>(null);
  const [resultFingerprint, setResultFingerprint] = React.useState<string | null>(null);
  const controllerRef = React.useRef(new RuleSetTestRequestController());
  const committedFingerprintRef = React.useRef("");

  const selection: RuleSetTestSelection = {
    titleId: selectedTitle?.id ?? null,
    episodeId: episodeId || null,
    releaseName,
    sizeGib,
  };
  const episodic = titleIsEpisodic(selectedTitle);
  const fingerprint = ruleSetTestFingerprint(
    draft,
    selection,
    editRuleSetId,
    copySourceRuleSetId,
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
      setCollections([]); setCollectionId(""); setEpisodes([]); setEpisodeId(""); return;
    }
    let cancelled = false;
    void (async () => {
      setLoadingEpisodes(true);
      try {
        const { data, error: collectionsError } = await client.query(ruleSetTestTitleCollectionsQuery, { id: selectedTitle.id }).toPromise();
        if (collectionsError) throw collectionsError;
        const next = (data?.title?.collections ?? []) as Collection[];
        if (!cancelled) { setCollections(next); setCollectionId(next[0]?.id ?? ""); }
      } catch (loadError) {
        if (!cancelled) setError(loadError instanceof Error ? loadError.message : "Unable to load seasons.");
      } finally {
        if (!cancelled) setLoadingEpisodes(false);
      }
    })();
    return () => { cancelled = true; };
  }, [client, episodic, selectedTitle]);

  React.useEffect(() => {
    if (!collectionId) { setEpisodes([]); setEpisodeId(""); setLoadingEpisodes(false); return; }
    let cancelled = false;
    void (async () => {
      setLoadingEpisodes(true); setEpisodeId("");
      try {
        const { data, error: episodesError } = await client.query(seriesCollectionEpisodesQuery, { id: collectionId }).toPromise();
        if (episodesError) throw episodesError;
        if (!cancelled) setEpisodes((data?.collectionById?.episodes ?? []) as Episode[]);
      } catch (loadError) {
        if (!cancelled) setError(loadError instanceof Error ? loadError.message : "Unable to load episodes.");
      } finally {
        if (!cancelled) setLoadingEpisodes(false);
      }
    })();
    return () => { cancelled = true; };
  }, [client, collectionId]);

  const clearEpisodeChoices = () => { setCollections([]); setCollectionId(""); setEpisodes([]); setEpisodeId(""); };
  const handleSelectedTitleChange = (title: TitleRecord | null) => {
    setSelectedTitle(title);
    clearEpisodeChoices();
    setLoadingEpisodes(Boolean(title && titleIsEpisodic(title)));
    setError(null);
  };

  const test = async () => {
    if (!canTest || !selectedTitle) return;
    const size = sizeBytesFromGib(sizeGib);
    if ("error" in size) { setError(size.error); return; }
    const request = controllerRef.current.begin();
    if (request == null) return;
    setTesting(true); setError(null);
    try {
      const { data, error: mutationError } = await client.mutation(testRuleSetMutation, { input: { draft, editRuleSetId: editRuleSetId || undefined, copySourceRuleSetId: copySourceRuleSetId || undefined, copyDisablesSource: Boolean(copySourceRuleSetId), titleId: selectedTitle.id, episodeId: episodic ? episodeId || undefined : undefined, releaseName: releaseName.trim(), sizeBytes: size.value } }).toPromise();
      if (mutationError) throw mutationError;
      if (!shouldApplyRuleSetTestResponse(request, controllerRef.current.isCurrent(request) ? request : -1, fingerprint, committedFingerprintRef.current)) return;
      setResult((data?.testRuleSet ?? null) as PreviewResult | null); setResultFingerprint(fingerprint);
    } catch (testError) {
      if (shouldApplyRuleSetTestResponse(request, controllerRef.current.isCurrent(request) ? request : -1, fingerprint, committedFingerprintRef.current)) setError(testError instanceof Error ? testError.message : "Scoring preview failed.");
    } finally {
      if (controllerRef.current.finish(request)) setTesting(false);
    }
  };

  const groupedRuleSets = React.useMemo(() => {
    const groups = new Map<string, PreviewRuleSet[]>();
    for (const item of result?.ruleSets ?? []) { const origin = item.origin || "Rules"; groups.set(origin, [...(groups.get(origin) ?? []), item]); }
    return [...groups.entries()];
  }, [result]);

  return <Collapsible
    open={open}
    onOpenChange={setOpen}
    className="rounded border border-border"
  >
    <CollapsibleTrigger className="flex w-full items-center justify-between px-3 py-2 text-left font-medium hover:bg-muted/50">
      <span>Test scoring</span>
      <span className="text-xs text-muted-foreground">
        {open ? "Hide" : "Show"}
      </span>
    </CollapsibleTrigger>
    <CollapsibleContent className="border-t border-border px-3 py-3">
      <p className="mb-3 text-xs text-muted-foreground">
        Scoring preview only. This does not save this draft, admit a download, or create history.
      </p>
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
        <div>
          <Label htmlFor="settings-rule-test-release" className="mb-1 block">Release name</Label>
          <Input id="settings-rule-test-release" value={releaseName} onChange={(event) => setReleaseName(event.target.value)} placeholder="Example.Show.S01E01.1080p.WEB-DL" />
        </div>
        <div>
          <Label htmlFor="settings-rule-test-size" className="mb-1 block">Size (GiB, optional)</Label>
          <Input id="settings-rule-test-size" inputMode="decimal" value={sizeGib} onChange={(event) => setSizeGib(event.target.value)} placeholder="Unknown" />
        </div>
        {episodic ? <><div><Label htmlFor="settings-rule-test-season" className="mb-1 block">Season</Label><select id="settings-rule-test-season" className="h-9 w-full rounded border border-input bg-background px-2 text-sm" value={collectionId} onChange={(event) => { setCollectionId(event.target.value); setEpisodes([]); setEpisodeId(""); setLoadingEpisodes(true); }} disabled={loadingEpisodes}><option value="">Select a season</option>{collections.map((collection) => <option key={collection.id} value={collection.id}>{collectionLabel(collection)}</option>)}</select></div><div><Label htmlFor="settings-rule-test-episode" className="mb-1 block">Episode</Label><select id="settings-rule-test-episode" className="h-9 w-full rounded border border-input bg-background px-2 text-sm" value={episodeId} onChange={(event) => setEpisodeId(event.target.value)} disabled={loadingEpisodes || !collectionId}><option value="">Select an episode</option>{episodes.map((episode) => <option key={episode.id} value={episode.id}>{episodeLabel(episode)}</option>)}</select></div></> : null}
      </div>
      {selectedTitle ? <p className="mt-2 text-xs text-muted-foreground">Selected: {selectedTitle.name} · {selectedTitle.libraryName || selectedTitle.libraryId}{loadingEpisodes ? " · Loading episodes…" : ""}</p> : null}
      <div className="mt-3 flex items-center gap-3"><Button type="button" onClick={() => void test()} disabled={!canTest}>{testing ? "Testing…" : "Test"}</Button>{stale ? <span className="text-xs text-[var(--scry-warning-text)]">Inputs changed; result is stale.</span> : null}</div>
      {error ? <div className="mt-3 rounded border border-[var(--scry-danger-border)] bg-[var(--scry-danger-bg)] px-3 py-2 text-sm text-[var(--scry-danger-text)]">{error}</div> : null}
      {result ? <div className="mt-3 space-y-3 rounded border border-border p-3" aria-live="polite">
        {incomplete ? <div className="rounded border border-[var(--scry-warning-border)] bg-[var(--scry-warning-bg)] px-3 py-2 text-sm text-[var(--scry-warning-text)]"><p className="font-medium">Incomplete scoring preview</p><p>This result includes evaluation errors, so it cannot determine download admission.</p></div> : null}
        <div><p className="font-medium">Scoring preview: {result.score ?? 0}</p><p className="text-xs text-muted-foreground">{incomplete ? "Evaluation completed with errors." : result.allowed ? "Allowed by the scoring policy" : "Not allowed by the scoring policy"}{result.minimumScoreMet === false ? " · Minimum score not met" : ""}</p></div>
        <div className="grid gap-2 text-xs md:grid-cols-2"><p><span className="font-medium">Resolved profile:</span> {result.profileName || "Unavailable"}</p>{result.context ? <p><span className="font-medium">Context:</span> {[result.context.titleName, result.context.libraryName, result.context.facet, result.context.language, result.context.episodeLabel].filter(Boolean).join(" · ") || "Unavailable"}</p> : null}<p><span className="font-medium">Tags:</span> {result.context?.tags?.length ? result.context.tags.join(", ") : "None"}</p></div>
        <div className="text-xs"><p className="font-medium">Parsed release details</p><dl className="grid grid-cols-2 gap-x-3 gap-y-1">{parsedLines(result.parsed).map(([label, value]) => <React.Fragment key={label}><dt className="text-muted-foreground">{label}</dt><dd>{value}</dd></React.Fragment>)}<dt className="text-muted-foreground">Size</dt><dd>{result.parsed?.sizeBytes == null ? "Unknown" : `${result.parsed.sizeBytes} bytes`}</dd></dl><p className="mt-1 text-muted-foreground">Listing metadata and file-probed facts are unavailable in this preview.</p></div>
        {result.draftContribution ? <div className="rounded bg-muted/50 px-2 py-2 text-xs"><p className="font-medium">Current draft: {formatSignedScore(result.draftContribution.score ?? 0)}</p><p>{result.draftContribution.message || (result.draftContribution.enabled === false ? "Draft is disabled." : result.draftContribution.applies === false ? "Draft does not apply to this title facet." : result.draftContribution.matched ? "Draft matched." : "Draft did not match.")}</p></div> : null}
        {groupedRuleSets.map(([origin, entries]) => <div key={origin} className="text-xs"><p className="font-medium">{origin}</p><ul className="mt-1 space-y-2">{entries.map((entry, index) => <li key={`${entry.ruleSetId || entry.ruleSetName || index}`}><div className="flex justify-between gap-2"><span>{entry.ruleSetName || "Unnamed rule"}{entry.isDraft ? " (draft)" : ""}{entry.messages?.length ? ` — ${entry.messages.join("; ")}` : ""}</span><span>{entry.matched ? formatSignedScore(entry.score ?? 0) : "No match"}</span></div>{entry.entries?.length ? <ul className="mt-1 space-y-1 border-l border-border pl-2 text-muted-foreground">{entry.entries.map((scoreEntry, entryIndex) => <li key={`${scoreEntry.code}-${entryIndex}`} className="flex justify-between gap-2"><span>{scoreEntry.code}</span><span>{scoringEntryText(scoreEntry, t)}</span></li>)}</ul> : null}</li>)}</ul></div>)}
        {result.errors?.length ? <div className="rounded border border-[var(--scry-danger-border)] bg-[var(--scry-danger-bg)] px-2 py-2 text-xs text-[var(--scry-danger-text)]"><p className="font-medium">Evaluation errors</p><ul className="list-disc pl-4">{result.errors.map((item, index) => <li key={`${item.code || "error"}-${index}`}>{item.message}</li>)}</ul></div> : null}
      </div> : null}
    </CollapsibleContent>
  </Collapsible>;
}
