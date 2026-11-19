import * as React from "react";

import { myListPolicyQuery } from "@/lib/graphql/queries";
import { backendClient } from "@/lib/graphql/urql-client";

// One read per member per page load, shared by every consumer. A failed read
// is not kept, so the next mount asks again.
const answers = new Map<string, boolean>();
const pending = new Map<string, Promise<boolean>>();

function readPersonalListsRefused(userId: string): Promise<boolean> {
  const inFlight = pending.get(userId);
  if (inFlight) return inFlight;
  const request = backendClient
    .query<{ myListPolicy?: string | null }>(
      myListPolicyQuery,
      {},
      { requestPolicy: "network-only" },
    )
    .toPromise()
    .then(({ data, error }) => {
      if (error) throw error;
      const refused = data?.myListPolicy === "NONE";
      answers.set(userId, refused);
      return refused;
    })
    .finally(() => {
      pending.delete(userId);
    });
  pending.set(userId, request);
  return request;
}

/**
 * Whether an admin has turned off this member's lists. Until the answer
 * arrives, and when it cannot be read, the member is treated as allowed so
 * the Lists entry never flickers away for someone who may use it.
 */
export function usePersonalListsRefused(userId: string): boolean {
  const [refused, setRefused] = React.useState(
    () => answers.get(userId) ?? false,
  );
  React.useEffect(() => {
    const known = answers.get(userId);
    if (known !== undefined) {
      setRefused(known);
      return;
    }
    setRefused(false);
    let active = true;
    readPersonalListsRefused(userId).then(
      (value) => {
        if (active) setRefused(value);
      },
      (error: unknown) => {
        console.warn("Failed to read the member's list policy", error);
      },
    );
    return () => {
      active = false;
    };
  }, [userId]);
  return refused;
}
