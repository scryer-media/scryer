import { createContext, useContext, useEffect, type ReactNode } from "react";
import { Provider } from "urql";
import { backendClient, smgClient, setGraphqlLanguage } from "@/lib/graphql/urql-client";
import type { Client } from "@urql/core";

// ---------------------------------------------------------------------------
// SMG client context — secondary client for metadata gateway queries
// ---------------------------------------------------------------------------

const SmgClientContext = createContext<Client>(smgClient);

export function useSmgClient(): Client {
  return useContext(SmgClientContext);
}

// ---------------------------------------------------------------------------
// Combined provider — provides both backend + SMG clients
// ---------------------------------------------------------------------------

export function ScryerGraphqlProvider({
  language,
  children,
}: {
  language: string;
  children: ReactNode;
}) {
  useEffect(() => {
    setGraphqlLanguage(language);
  }, [language]);

  return (
    <Provider value={backendClient}>
      <SmgClientContext.Provider value={smgClient}>
        {children}
      </SmgClientContext.Provider>
    </Provider>
  );
}
