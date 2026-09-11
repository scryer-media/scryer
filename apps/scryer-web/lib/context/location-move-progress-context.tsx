import { createContext, useContext } from "react";

export type LocationMoveProgressContextValue = {
  /** Operations whose toast is live or backgrounded on this page load. */
  trackedOperationIds: string[];
  /** Surface an operation's toast the moment its start is accepted. */
  trackOperation: (operationId: string) => void;
};

export const LocationMoveProgressContext =
  createContext<LocationMoveProgressContextValue | null>(null);

/**
 * Null outside the provider: the location dialogs render without one in
 * tests, and a start there is simply not toasted.
 */
export function useOptionalLocationMoveProgress(): LocationMoveProgressContextValue | null {
  return useContext(LocationMoveProgressContext);
}
