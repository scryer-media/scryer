import { createGraphiQLFetcher, type Fetcher } from "@graphiql/toolkit";
import { GraphQLError, getOperationAST, parse } from "graphql";
import { createClient } from "graphql-ws";

export type ApiExplorerMode = "api-key" | "oauth";
export const API_EXPLORER_MODE_HEADER = "x-scryer-api-explorer-mode";

export function explorerHeaders(
  headers: HeadersInit | undefined,
  token: string | null,
  mode: ApiExplorerMode,
): Headers {
  const result = new Headers(headers);
  result.delete("authorization");
  result.delete("x-scryer-web-client");
  if (token) result.set("authorization", `Bearer ${token}`);
  result.set(API_EXPLORER_MODE_HEADER, mode);
  return result;
}

export function createApiExplorerTransport(options: {
  mode: ApiExplorerMode;
  url: string;
  fetch: typeof fetch;
  getToken: () => string | null;
  getProof: () => Promise<string | null>;
  onUnavailable: () => void;
}) {
  const controller = new AbortController();
  const url = new URL(options.url);
  url.protocol = url.protocol === "https:" ? "wss:" : "ws:";
  url.pathname = url.pathname.replace(/\/+$/, "") + "/ws";
  const wsClient = options.mode === "oauth" ? createClient({
    url: url.toString(),
    retryAttempts: 0,
    connectionParams: async () => {
      const token = options.getToken();
      const proof = token ? null : await options.getProof();
      return {
        [API_EXPLORER_MODE_HEADER]: options.mode,
        ...(token ? { Authorization: `Bearer ${token}` } : {}),
        ...(proof ? { authlessWebClientProof: proof } : {}),
      };
    },
    on: {
      closed: () => {
        if (!controller.signal.aborted) options.onUnavailable();
      },
    },
  }) : undefined;
  const execute = createGraphiQLFetcher({
    url: options.url,
    wsClient,
    enableIncrementalDelivery: false,
    fetch: async (input, init) => {
      controller.signal.throwIfAborted();
      const response = await options.fetch(input, {
        ...init,
        credentials: "include",
        headers: explorerHeaders(init?.headers, options.getToken(), options.mode),
        signal: init?.signal
          ? AbortSignal.any([controller.signal, init.signal])
          : controller.signal,
      });
      if (response.status === 403) {
        const result = await response.clone().json().catch(() => null);
        if (result?.errors?.some((error: { extensions?: { code?: string } }) =>
          error.extensions?.code === "API_EXPLORER_UNAVAILABLE")) {
          options.onUnavailable();
        }
      }
      return response;
    },
  });
  const fetcher: Fetcher = (params, opts) => {
    controller.signal.throwIfAborted();
    const documentAST = parse(params.query);
    if (options.mode === "api-key" &&
        getOperationAST(documentAST, params.operationName)?.operation === "subscription") {
      return { errors: [new GraphQLError("API keys do not support subscriptions. Select OAuth App to test subscriptions.")] };
    }
    return execute(params, { ...opts, documentAST });
  };
  return {
    fetcher,
    dispose() {
      controller.abort();
      void wsClient?.dispose();
    },
  };
}
