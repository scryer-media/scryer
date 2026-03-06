import { createContext, useContext } from "react";
import type { Translate } from "@/components/root/types";

export const TranslateContext = createContext<Translate | null>(null);

export function useTranslate(): Translate {
  const t = useContext(TranslateContext);
  if (!t) throw new Error("useTranslate must be used within TranslateContext.Provider");
  return t;
}
