import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { Loader2, Search } from "lucide-react";

type Translate = (
  key: string,
  values?: Record<string, string | number | boolean | null | undefined>,
) => string;

export type CutoffUnmetItem = {
  id: string;
  name: string;
  facet: string;
  posterUrl?: string | null;
  currentTier: string;
  targetTier: string;
};

type CutoffUnmetViewState = {
  t: Translate;
  items: CutoffUnmetItem[];
  loading: boolean;
  facetFilter: string | undefined;
  setFacetFilter: (v: string | undefined) => void;
  searchingId: string | null;
  bulkSearching: boolean;
  bulkProgress: { current: number; total: number } | null;
  triggerSearch: (item: CutoffUnmetItem) => Promise<void>;
  triggerBulkSearch: () => void;
  cancelBulkSearch: () => void;
};

function qualityBadge(tier: string, variant: "current" | "target") {
  const cls =
    variant === "current"
      ? "bg-amber-500/20 text-amber-400"
      : "bg-green-500/20 text-green-400";
  return (
    <span
      className={`inline-block rounded px-2 py-0.5 text-xs font-medium ${cls}`}
    >
      {tier}
    </span>
  );
}

export function CutoffUnmetView({ state }: { state: CutoffUnmetViewState }) {
  const {
    t,
    items,
    loading,
    facetFilter,
    setFacetFilter,
    searchingId,
    bulkSearching,
    bulkProgress,
    triggerSearch,
    triggerBulkSearch,
    cancelBulkSearch,
  } = state;

  const filtered = facetFilter
    ? items.filter((i) => i.facet === facetFilter)
    : items;

  return (
    <Card>
      <CardHeader>
        <div className="flex items-center justify-between">
          <CardTitle>{t("cutoff.title")}</CardTitle>
          <div className="flex items-center gap-2">
            {bulkSearching && bulkProgress ? (
              <>
                <span className="text-sm text-muted-foreground">
                  {t("cutoff.searchProgress", {
                    current: bulkProgress.current,
                    total: bulkProgress.total,
                  })}
                </span>
                <Button size="sm" variant="destructive" onClick={cancelBulkSearch}>
                  {t("label.cancel")}
                </Button>
              </>
            ) : (
              <Button
                size="sm"
                onClick={triggerBulkSearch}
                disabled={filtered.length === 0 || loading}
              >
                <Search className="mr-1 h-3 w-3" />
                {t("cutoff.searchAll")}
              </Button>
            )}
          </div>
        </div>
      </CardHeader>
      <CardContent>
        <div className="mb-4 flex flex-wrap gap-3">
          <Select
            value={facetFilter ?? "__all__"}
            onValueChange={(v) => setFacetFilter(v === "__all__" ? undefined : v)}
          >
            <SelectTrigger className="w-[150px]">
              <SelectValue placeholder={t("cutoff.filterFacet")} />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="__all__">{t("cutoff.allFacets")}</SelectItem>
              <SelectItem value="movie">movie</SelectItem>
              <SelectItem value="tv">tv</SelectItem>
              <SelectItem value="anime">anime</SelectItem>
            </SelectContent>
          </Select>

          <span className="self-center text-sm text-muted-foreground">
            {t("cutoff.totalCount", { count: filtered.length })}
          </span>
        </div>

        <div className="overflow-x-auto">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead className="w-10" />
                <TableHead>{t("cutoff.colTitle")}</TableHead>
                <TableHead>{t("cutoff.colFacet")}</TableHead>
                <TableHead>{t("cutoff.colCurrentQuality")}</TableHead>
                <TableHead>{t("cutoff.colTargetQuality")}</TableHead>
                <TableHead>{t("cutoff.colActions")}</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {filtered.map((item) => (
                <TableRow key={item.id}>
                  <TableCell>
                    {item.posterUrl ? (
                      <img
                        src={item.posterUrl}
                        alt={item.name}
                        className="h-8 w-6 rounded object-cover"
                        loading="lazy"
                      />
                    ) : (
                      <div className="h-8 w-6 rounded bg-muted" />
                    )}
                  </TableCell>
                  <TableCell className="max-w-[250px] truncate text-sm font-medium">
                    {item.name}
                  </TableCell>
                  <TableCell className="text-sm">{item.facet}</TableCell>
                  <TableCell>{qualityBadge(item.currentTier, "current")}</TableCell>
                  <TableCell>{qualityBadge(item.targetTier, "target")}</TableCell>
                  <TableCell>
                    <Button
                      size="icon"
                      variant="ghost"
                      className="h-7 w-7"
                      disabled={searchingId === item.id || bulkSearching}
                      onClick={() => void triggerSearch(item)}
                    >
                      {searchingId === item.id ? (
                        <Loader2 className="h-3.5 w-3.5 animate-spin" />
                      ) : (
                        <Search className="h-3.5 w-3.5" />
                      )}
                    </Button>
                  </TableCell>
                </TableRow>
              ))}
              {filtered.length === 0 && !loading && (
                <TableRow>
                  <TableCell colSpan={6} className="text-center text-muted-foreground">
                    {t("cutoff.noItems")}
                  </TableCell>
                </TableRow>
              )}
            </TableBody>
          </Table>
        </div>
      </CardContent>
    </Card>
  );
}
