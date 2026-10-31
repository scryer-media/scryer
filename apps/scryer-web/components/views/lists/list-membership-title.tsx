import * as React from "react";

import {
  Popover,
  PopoverAnchor,
  PopoverContent,
} from "@/components/ui/popover";
import { facetById } from "@/lib/facets/registry";
import { useTranslate } from "@/lib/context/translate-context";
import { useIsMobile } from "@/lib/hooks/use-mobile";
import type { ListMembership } from "@/lib/types/lists";
import {
  listKindLabelKey,
  listTitleWithoutYear,
  listMembershipMetadataLookup,
  listMembershipStateLabelKey,
  listMembershipTitleHref,
} from "@/lib/utils/lists";

import {
  LIST_TITLE_FLYOUT_CLASS,
  ListTitleFlyoutBody,
} from "./list-title-flyout";
import { useListTitleMetadata } from "./use-list-title-metadata";

const OPEN_DELAY_MS = 200;
const CLOSE_DELAY_MS = 180;

/**
 * A list entry's title: a link when the title is in a library, and for every
 * entry a flyout to its left with what the metadata service knows of it.
 */
export function ListMembershipTitle({
  membership,
}: {
  membership: ListMembership;
}) {
  const t = useTranslate();
  const isMobile = useIsMobile();
  const [open, setOpen] = React.useState(false);
  // The flyout opens with what the list says and fills in when the service answers;
  // an entry the service cannot describe keeps the plain flyout.
  const [metadata, requestMetadata] = useListTitleMetadata(
    listMembershipMetadataLookup(membership),
  );
  const timer = React.useRef<number | null>(null);
  const label = listTitleWithoutYear(
    membership.displayTitle ?? membership.itemKey,
    membership.year,
  );
  const href = listMembershipTitleHref(
    membership.kind,
    membership.titleId,
    membership.seriesMovieLinkId,
  );
  const FacetIcon = facetById(membership.kind)?.icon;

  const clearTimer = React.useCallback(() => {
    if (timer.current !== null) {
      window.clearTimeout(timer.current);
      timer.current = null;
    }
  }, []);
  React.useEffect(() => clearTimer, [clearTimer]);

  const settle = (next: boolean, delay: number) => {
    clearTimer();
    timer.current = window.setTimeout(() => {
      setOpen(next);
      timer.current = null;
    }, delay);
  };
  const show = () => {
    if (isMobile) return;
    requestMetadata();
    settle(true, OPEN_DELAY_MS);
  };
  const hide = () => settle(false, CLOSE_DELAY_MS);

  const hoverProps = {
    onMouseEnter: show,
    onMouseLeave: hide,
    onFocus: (event: React.FocusEvent<HTMLElement>) => {
      if (event.currentTarget.matches(":focus-visible")) show();
    },
    onBlur: hide,
  };

  return (
    <Popover
      open={open && !isMobile}
      onOpenChange={(next) => {
        clearTimer();
        setOpen(next);
      }}
    >
      <PopoverAnchor asChild>
        {href ? (
          <a
            href={href}
            data-ui="list-membership-title"
            className="rounded-sm hover:underline focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--scry-focus)]"
            {...hoverProps}
          >
            {label}
          </a>
        ) : (
          <span data-ui="list-membership-title" {...hoverProps}>
            {label}
          </span>
        )}
      </PopoverAnchor>
      <PopoverContent
        side="left"
        align="center"
        sideOffset={12}
        collisionPadding={12}
        aria-label={label}
        className={LIST_TITLE_FLYOUT_CLASS}
        onOpenAutoFocus={(event) => event.preventDefault()}
        onCloseAutoFocus={(event) => event.preventDefault()}
        onMouseEnter={clearTimer}
        onMouseLeave={hide}
      >
        <ListTitleFlyoutBody
          title={label}
          kindIcon={
            FacetIcon ? (
              <FacetIcon className="h-4 w-4" aria-hidden="true" />
            ) : null
          }
          kindLabel={t(listKindLabelKey(membership.kind))}
          year={membership.year}
          runtimeMinutes={metadata?.runtimeMinutes}
          posterUrl={metadata?.posterUrl}
          backgroundUrl={metadata?.backgroundUrl}
          externalRatings={metadata?.externalRatings}
          overview={metadata?.overview}
          genres={metadata?.genresAndThemes}
          footer={t(listMembershipStateLabelKey(membership.state))}
        />
      </PopoverContent>
    </Popover>
  );
}
