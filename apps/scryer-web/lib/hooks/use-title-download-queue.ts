import { useEffect, useRef, useState } from "react";

import { downloadQueueSubscription } from "@/lib/graphql/queries";
import { useDeferredWsSubscription } from "@/lib/hooks/use-deferred-ws-subscription";
import type { DownloadQueueItem } from "@/lib/types/download-queue";
import {
  mergeAuthoritativeQueueItems,
  mergeLiveQueueItems,
  sortDownloadQueueItems,
} from "@/lib/utils/download-queue";

type UseTitleDownloadQueueArgs = {
  enabled: boolean;
  titleId?: string | null;
  initialItems: DownloadQueueItem[];
};

export function useTitleDownloadQueue({
  enabled,
  titleId,
  initialItems,
}: UseTitleDownloadQueueArgs): DownloadQueueItem[] {
  const [queueItems, setQueueItems] = useState<DownloadQueueItem[]>(() =>
    sortDownloadQueueItems(initialItems),
  );
  const streamStartedRef = useRef(false);
  const activeTitleIdRef = useRef<string | null>(titleId ?? null);

  useEffect(() => {
    const nextTitleId = titleId ?? null;
    const titleChanged = activeTitleIdRef.current !== nextTitleId;
    activeTitleIdRef.current = nextTitleId;

    if (!nextTitleId) {
      streamStartedRef.current = false;
      setQueueItems([]);
      return;
    }

    if (titleChanged) {
      streamStartedRef.current = false;
    }

    setQueueItems((previousItems) => {
      const authoritativeItems = sortDownloadQueueItems(initialItems);
      if (titleChanged || !streamStartedRef.current) {
        return authoritativeItems;
      }
      return mergeAuthoritativeQueueItems(authoritativeItems, previousItems);
    });
  }, [initialItems, titleId]);

  useDeferredWsSubscription<{ data?: { downloadQueue?: DownloadQueueItem[] } }>(
    {
      enabled: enabled && Boolean(titleId),
      requestKey: `titleDownloadQueue:${titleId ?? "none"}`,
      request: {
        query: downloadQueueSubscription,
        variables: {
          titleId,
          includeAllActivity: true,
          includeImportActivity: true,
          activityFilter: "ALL",
        },
      },
      onNext(result) {
        const items = result.data?.downloadQueue;
        if (!items) {
          return;
        }
        streamStartedRef.current = true;
        setQueueItems((previousItems) =>
          mergeLiveQueueItems(items, previousItems),
        );
      },
      onError(error) {
        console.error("[title-download-queue] subscription error:", error);
      },
    },
  );

  return queueItems;
}
