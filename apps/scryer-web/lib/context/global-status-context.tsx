import { createContext, useContext } from "react";

export const GlobalStatusContext = createContext<((status: string) => void) | null>(null);

export function useGlobalStatus(): (status: string) => void {
  const fn = useContext(GlobalStatusContext);
  if (!fn) throw new Error("useGlobalStatus must be used within GlobalStatusContext.Provider");
  return fn;
}
