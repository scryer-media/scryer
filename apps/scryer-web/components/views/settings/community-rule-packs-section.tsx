import * as React from "react";
import { useClient } from "urql";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { TableDisclosureButton } from "@/components/ui/table-disclosure-button";
import { useTranslate } from "@/lib/context/translate-context";
import {
  rulePackRegistryQuery,
  rulePackTemplatesQuery,
} from "@/lib/graphql/queries";
import { selectorId } from "@/lib/utils/dom-ids";

type CommunityPack = {
  id: string;
  name: string;
  description: string;
  author: string;
  version: string;
  customizable?: boolean | null;
};

type CommunityTemplate = {
  id: string;
  title: string;
  description: string;
  category: string;
  appliedFacets: string[];
};

type PackTemplates =
  { status: "loading" } | { status: "ready"; templates: CommunityTemplate[] };

/**
 * The registry's scoring rule packs that are not installed yet. Opening a row
 * lists the pack's rules, so the operator picks which to enable before
 * installing it.
 */
export function CommunityRulePacksSection({
  installedPackIds,
  canManage,
  installingPackId,
  onInstall,
}: {
  installedPackIds: readonly string[];
  canManage: boolean;
  installingPackId: string | null;
  onInstall: (packId: string, templateIds: string[]) => Promise<void> | void;
}) {
  const t = useTranslate();
  const client = useClient();
  const [packs, setPacks] = React.useState<CommunityPack[]>([]);
  const [loaded, setLoaded] = React.useState(false);
  const [expandedPackIds, setExpandedPackIds] = React.useState<Set<string>>(
    () => new Set(),
  );
  const [templatesByPack, setTemplatesByPack] = React.useState<
    Map<string, PackTemplates>
  >(() => new Map());
  const [selectedByPack, setSelectedByPack] = React.useState<
    Map<string, string[]>
  >(() => new Map());

  React.useEffect(() => {
    let cancelled = false;
    client
      .query(rulePackRegistryQuery, {})
      .toPromise()
      .then(({ data }) => {
        if (cancelled) return;
        setPacks(data?.rulePackRegistry ?? []);
        setLoaded(true);
      })
      .catch(() => {
        if (!cancelled) setLoaded(true);
      });
    return () => {
      cancelled = true;
    };
  }, [client]);

  const loadTemplates = React.useCallback(
    (packId: string) => {
      if (templatesByPack.has(packId)) return;
      setTemplatesByPack((current) =>
        new Map(current).set(packId, { status: "loading" }),
      );
      client
        .query(rulePackTemplatesQuery, { packId })
        .toPromise()
        .then(({ data }) => data?.rulePackTemplates ?? [])
        .catch(() => [])
        .then((templates: CommunityTemplate[]) =>
          setTemplatesByPack((current) =>
            new Map(current).set(packId, { status: "ready", templates }),
          ),
        );
    },
    [client, templatesByPack],
  );

  const setExpanded = (packId: string, expanded: boolean) => {
    if (expanded) loadTemplates(packId);
    setExpandedPackIds((current) => {
      const next = new Set(current);
      if (expanded) next.add(packId);
      else next.delete(packId);
      return next;
    });
  };

  const installed = new Set(installedPackIds);
  const available = packs.filter((pack) => !installed.has(pack.id));

  return (
    <section id="settings-community-rule-packs" className="space-y-3">
      <div>
        <h3 className="text-base font-semibold">
          {t("settings.communityRulePacks")}
        </h3>
        <p className="text-xs text-muted-foreground">
          {t("settings.communityRulePacksDescription")}
        </p>
      </div>
      <div className="overflow-x-auto rounded border border-border bg-card">
        <Table
          layout="fixed"
          className="min-w-[860px]"
          aria-label={t("settings.communityRulePacks")}
        >
          <TableHeader>
            <TableRow>
              <TableHead className="w-10" />
              <TableHead>{t("uiText.pack")}</TableHead>
              <TableHead className="w-44">
                {t("settings.rulePackAuthor")}
              </TableHead>
              <TableHead className="w-28">{t("label.version")}</TableHead>
              <TableHead className="w-36 text-right">
                {t("label.actions")}
              </TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {!loaded ? (
              <TableRow>
                <TableCell colSpan={5} className="text-muted-foreground">
                  {t("label.loading")}
                </TableCell>
              </TableRow>
            ) : available.length === 0 ? (
              <TableRow>
                <TableCell colSpan={5} className="text-muted-foreground">
                  {packs.length === 0
                    ? t("settings.ruleLibraryCommunityEmpty")
                    : t("settings.communityRulePacksAllInstalled")}
                </TableCell>
              </TableRow>
            ) : (
              available.map((pack) => (
                <CommunityPackRow
                  key={pack.id}
                  pack={pack}
                  expanded={expandedPackIds.has(pack.id)}
                  onExpandedChange={(next) => setExpanded(pack.id, next)}
                  templates={templatesByPack.get(pack.id)}
                  selectedTemplateIds={selectedByPack.get(pack.id) ?? []}
                  onSelectedTemplateIdsChange={(ids) =>
                    setSelectedByPack((current) =>
                      new Map(current).set(pack.id, ids),
                    )
                  }
                  canManage={canManage}
                  installing={installingPackId === pack.id}
                  installDisabled={installingPackId !== null}
                  onInstall={onInstall}
                />
              ))
            )}
          </TableBody>
        </Table>
      </div>
    </section>
  );
}

function CommunityPackRow({
  pack,
  expanded,
  onExpandedChange,
  templates,
  selectedTemplateIds,
  onSelectedTemplateIdsChange,
  canManage,
  installing,
  installDisabled,
  onInstall,
}: {
  pack: CommunityPack;
  expanded: boolean;
  onExpandedChange: (expanded: boolean) => void;
  templates: PackTemplates | undefined;
  selectedTemplateIds: string[];
  onSelectedTemplateIdsChange: (ids: string[]) => void;
  canManage: boolean;
  installing: boolean;
  installDisabled: boolean;
  onInstall: (packId: string, templateIds: string[]) => Promise<void> | void;
}) {
  const t = useTranslate();
  const rowId = selectorId("settings-community-rule-pack", pack.id);
  const detailId = `${rowId}-details`;
  const customizable = pack.customizable !== false;
  const ready = templates?.status === "ready" ? templates.templates : null;

  return (
    <>
      <TableRow
        id={rowId}
        data-ui="settings-table-row"
        className="hover:bg-[var(--scry-rowHover)] [&>td]:align-middle"
      >
        <TableCell>
          <TableDisclosureButton
            id={`${rowId}-toggle`}
            expanded={expanded}
            aria-controls={detailId}
            aria-label={pack.name}
            onClick={() => onExpandedChange(!expanded)}
          />
        </TableCell>
        <TableCell>
          <span className="flex items-center gap-2">
            <span className="truncate font-medium">{pack.name}</span>
            {!customizable ? (
              <Badge tone="neutral" className="shrink-0 text-[10px]">
                {t("settings.managedIndexerBadge")}
              </Badge>
            ) : null}
          </span>
          {pack.description ? (
            <span className="mt-0.5 block text-xs text-muted-foreground line-clamp-2">
              {pack.description}
            </span>
          ) : null}
        </TableCell>
        <TableCell className="truncate text-muted-foreground">
          {pack.author || "—"}
        </TableCell>
        <TableCell className="tabular-nums text-muted-foreground">
          {pack.version}
        </TableCell>
        <TableCell className="text-right">
          {canManage && !expanded ? (
            <Button
              id={`${rowId}-choose`}
              type="button"
              size="sm"
              variant="secondary"
              disabled={installDisabled}
              onClick={() => onExpandedChange(true)}
            >
              {t("settings.rulePackInstall")}
            </Button>
          ) : null}
        </TableCell>
      </TableRow>
      {expanded ? (
        <TableRow id={detailId} data-ui="settings-table-row">
          <TableCell colSpan={5} className="p-4">
            <div className="@container space-y-3">
              {ready === null ? (
                <p className="text-sm text-muted-foreground">
                  {t("label.loading")}
                </p>
              ) : (
                <>
                  <div className="space-y-1 text-xs text-muted-foreground">
                    <p>
                      {t(
                        customizable
                          ? "rules.packChooseCustomizable"
                          : "rules.packChooseManaged",
                      )}
                    </p>
                    <p>
                      {t(
                        "uiText.existingCustomCopiesAreNotAdoptedOrDisabledWhenThisPackIsInstalled",
                      )}
                    </p>
                  </div>
                  <div className="grid grid-cols-1 gap-2 @[560px]:grid-cols-2 @[900px]:grid-cols-3">
                    {ready.map((template) => {
                      const checked = selectedTemplateIds.includes(template.id);
                      return (
                        <label
                          key={template.id}
                          className="flex cursor-pointer gap-3 rounded-lg border border-border bg-card/50 p-3"
                        >
                          <Checkbox
                            id={selectorId(
                              "settings-rules-library-template-install",
                              template.id,
                            )}
                            checked={checked}
                            disabled={!canManage}
                            onCheckedChange={(value) =>
                              onSelectedTemplateIdsChange(
                                value === true
                                  ? [...selectedTemplateIds, template.id]
                                  : selectedTemplateIds.filter(
                                      (id) => id !== template.id,
                                    ),
                              )
                            }
                          />
                          <span className="min-w-0">
                            <span className="block text-sm font-medium">
                              {template.title}
                            </span>
                            <span className="mt-1 block text-xs text-muted-foreground line-clamp-2">
                              {template.description}
                            </span>
                            <span className="mt-2 flex flex-wrap gap-1">
                              <Badge tone="neutral" className="text-[10px]">
                                {template.category}
                              </Badge>
                              {template.appliedFacets.map((facet) => (
                                <Badge
                                  key={facet}
                                  tone="info"
                                  className="text-[10px]"
                                >
                                  {facet}
                                </Badge>
                              ))}
                            </span>
                          </span>
                        </label>
                      );
                    })}
                  </div>
                  {canManage ? (
                    <div className="flex gap-2">
                      <Button
                        id={selectorId(
                          "settings-rules-library-pack-install",
                          pack.id,
                        )}
                        type="button"
                        disabled={installDisabled}
                        onClick={() =>
                          void onInstall(pack.id, selectedTemplateIds)
                        }
                      >
                        {installing
                          ? t("settings.rulePackInstalling")
                          : t("settings.rulePackInstall")}
                      </Button>
                      <Button
                        id={`${rowId}-cancel`}
                        type="button"
                        variant="secondary"
                        onClick={() => onExpandedChange(false)}
                      >
                        {t("label.cancel")}
                      </Button>
                    </div>
                  ) : null}
                </>
              )}
            </div>
          </TableCell>
        </TableRow>
      ) : null}
    </>
  );
}
