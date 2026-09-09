import * as React from "react";
import { useClient } from "urql";
import { TitleAutocompletePicker } from "@/components/common/title-autocomplete-picker";
import { SingleSelectField } from "@/components/ui/select";
import { useTranslate } from "@/lib/context/translate-context";
import {
  ruleSetTestTitleCollectionsQuery,
  seriesCollectionEpisodesQuery,
} from "@/lib/graphql/queries";
import type { TitleRecord } from "@/lib/types";
import type {
  MaintenanceRuleScope,
  MaintenanceTestSubject,
} from "@/lib/types/maintenance-rule-sets";

type CollectionOption = {
  id: string;
  label: string | null;
  collectionIndex: string;
  collectionType: string;
};
type EpisodeOption = {
  id: string;
  title: string | null;
  episodeNumber: string | null;
  absoluteNumber: string | null;
};

/** Choose catalog identities; preview resolves their ownership again. */
export function MaintenanceSubjectPicker({
  scope,
  onChange,
}: {
  scope: MaintenanceRuleScope;
  onChange: (
    subject: MaintenanceTestSubject | undefined,
    pending: boolean,
    active: boolean,
  ) => void;
}) {
  const t = useTranslate();
  const client = useClient();
  const [title, setTitle] = React.useState<TitleRecord | null>(null);
  const [collections, setCollections] = React.useState<CollectionOption[]>([]);
  const [collectionId, setCollectionId] = React.useState("");
  const [episodes, setEpisodes] = React.useState<EpisodeOption[]>([]);
  const [episodeId, setEpisodeId] = React.useState("");
  const [error, setError] = React.useState<string | null>(null);
  const [loading, setLoading] = React.useState(false);
  React.useEffect(() => {
    if (!title || scope === "TITLE") return;
    let cancelled = false;
    setLoading(true);
    void client
      .query(ruleSetTestTitleCollectionsQuery, { id: title.id })
      .toPromise()
      .then(({ data, error }) => {
        if (cancelled) return;
        if (error) {
          setError(error.message);
          return;
        }
        setCollections(
          ((data?.title?.collections ?? []) as CollectionOption[]).filter(
            (collection) =>
              scope === "EPISODE" ||
              ["season", "specials"].includes(
                collection.collectionType.toLowerCase(),
              ),
          ),
        );
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [client, title, scope]);
  React.useEffect(() => {
    if (!collectionId || scope !== "EPISODE") return;
    let cancelled = false;
    setLoading(true);
    void client
      .query(seriesCollectionEpisodesQuery, { id: collectionId })
      .toPromise()
      .then(({ data, error }) => {
        if (cancelled) return;
        if (error) {
          setError(error.message);
          return;
        }
        setEpisodes(data?.collectionById?.episodes ?? []);
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [client, collectionId, scope]);
  return (
    <div className="grid gap-3 md:grid-cols-3">
      <TitleAutocompletePicker
        selectedTitle={title}
        selectedTitleId={title?.id ?? null}
        ariaLabel={t("settings.maintenanceTestTitle")}
        placeholder={t("settings.maintenanceTestTitle")}
        onSelectedTitleChange={(next) => {
          setTitle(next);
          setCollectionId("");
          setEpisodeId("");
          setCollections([]);
          setEpisodes([]);
          setError(null);
          onChange(
            next && scope === "TITLE"
              ? { titleId: next.id, subjectId: next.id }
              : undefined,
            Boolean(next && scope !== "TITLE"),
            Boolean(next),
          );
        }}
      />
      {title && scope !== "TITLE" && (
        <SingleSelectField
          id="settings-maintenance-test-season"
          label={t("settings.maintenanceScopeSeason")}
          value={collectionId}
          disabled={loading}
          options={collections.map((collection) => ({
            value: collection.id,
            label:
              collection.label ??
              `${t("settings.maintenanceScopeSeason")} ${collection.collectionIndex}`,
          }))}
          onValueChange={(id) => {
            setCollectionId(id);
            setEpisodeId("");
            setEpisodes([]);
            setError(null);
            onChange(
              scope === "SEASON"
                ? { titleId: title.id, subjectId: id }
                : undefined,
              scope === "EPISODE",
              true,
            );
          }}
        />
      )}
      {title && collectionId && scope === "EPISODE" && (
        <SingleSelectField
          id="settings-maintenance-test-episode"
          label={t("settings.maintenanceScopeEpisode")}
          value={episodeId}
          disabled={loading}
          options={episodes.map((episode) => ({
            value: episode.id,
            label: `${episode.episodeNumber ?? episode.absoluteNumber ?? "?"}${episode.title ? ` · ${episode.title}` : ""}`,
          }))}
          onValueChange={(id) => {
            setEpisodeId(id);
            onChange({ titleId: title.id, subjectId: id }, false, true);
          }}
        />
      )}
      {error && (
        <p role="alert" className="text-sm text-destructive">
          {error}
        </p>
      )}
      {title &&
        scope !== "TITLE" &&
        !loading &&
        !error &&
        (collections.length === 0 ||
          (scope === "EPISODE" && collectionId && episodes.length === 0)) && (
          <p className="text-sm text-muted-foreground">
            {t("settings.maintenanceNoSubjects")}
          </p>
        )}
    </div>
  );
}
