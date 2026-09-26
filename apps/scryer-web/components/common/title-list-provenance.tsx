import * as React from "react";
import { ListPlus } from "lucide-react";
import { useClient } from "urql";

import { useExperimentalFeaturesEnabled } from "@/lib/context/instance-features-context";
import { useTranslate } from "@/lib/context/translate-context";
import { titleListMembershipsQuery } from "@/lib/graphql/queries";
import type { TitleListMembership } from "@/lib/types/lists";
import { cn } from "@/lib/utils";
import { titleListProvenance } from "@/lib/utils/lists";

type TitleListProvenanceProps = {
  titleId: string;
  variant?: "default" | "hero";
  className?: string;
};

/**
 * "Added by list" under a title's ratings, naming the public list that put
 * the title in the library, or a muted note once that list has dropped it.
 * Renders nothing for titles no list added.
 */
export function TitleListProvenance({ titleId, variant = "default", className }: TitleListProvenanceProps) {
  const client = useClient();
  const t = useTranslate();
  const experimentalFeaturesEnabled = useExperimentalFeaturesEnabled();
  const [memberships, setMemberships] = React.useState<TitleListMembership[]>([]);

  React.useEffect(() => {
    let cancelled = false;
    setMemberships([]);
    // Lists are experimental: nothing is asked for while the switch is off.
    if (!experimentalFeaturesEnabled) return;
    void client
      .query(titleListMembershipsQuery, { id: titleId })
      .toPromise()
      .then((result) => {
        if (cancelled || result.error) return;
        setMemberships((result.data?.title?.listMemberships ?? []) as TitleListMembership[]);
      });
    return () => {
      cancelled = true;
    };
  }, [client, experimentalFeaturesEnabled, titleId]);

  const provenance = titleListProvenance(memberships);
  if (!provenance) return null;

  const left = provenance.kind === "left";
  const tone =
    variant === "hero"
      ? left
        ? "text-[var(--scry-muted2)]"
        : "text-[#cfd7ee]"
      : left
        ? "text-muted-foreground/70"
        : "text-muted-foreground";

  return (
    <p
      id="title-list-provenance"
      data-state={provenance.kind}
      className={cn("flex min-w-0 items-center gap-1.5 text-[12px]", tone, className)}
    >
      <ListPlus className="h-3.5 w-3.5 flex-none" aria-hidden />
      <span className="min-w-0 truncate">
        {left
          ? t("title.listProvenance.left", { name: provenance.name })
          : t("title.listProvenance.added", { name: provenance.name })}
      </span>
    </p>
  );
}
