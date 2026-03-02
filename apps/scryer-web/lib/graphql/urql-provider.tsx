import { useEffect, type ReactNode } from "react";
import { Provider } from "urql";
import { backendClient, setGraphqlLanguage } from "@/lib/graphql/urql-client";

// ---------------------------------------------------------------------------
// Combined provider — provides the backend GraphQL client
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

  return <Provider value={backendClient}>{children}</Provider>;
}
