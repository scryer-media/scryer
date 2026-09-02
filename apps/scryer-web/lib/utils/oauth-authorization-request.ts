export type OAuthAuthorizationRequest = {
  responseType: string;
  clientId: string;
  redirectUri: string;
  codeChallenge: string;
  codeChallengeMethod: string;
  scope: string | null;
  state: string | null;
};

type GraphQlErrorLike = {
  extensions?: Record<string, unknown> | null;
};

type OAuthErrorLike = {
  response?: { status?: number } | null;
  graphQLErrors?: readonly GraphQlErrorLike[];
};

export function oauthAuthorizationRequestFromSearch(search: string): OAuthAuthorizationRequest {
  const params = new URLSearchParams(search);
  return {
    responseType: params.get("response_type") ?? "",
    clientId: params.get("client_id") ?? "",
    redirectUri: params.get("redirect_uri") ?? "",
    codeChallenge: params.get("code_challenge") ?? "",
    codeChallengeMethod: params.get("code_challenge_method") ?? "",
    scope: params.get("scope"),
    state: params.get("state"),
  };
}

export function isOAuthAuthenticationError(error: unknown): boolean {
  if (!error || typeof error !== "object") return false;
  const candidate = error as OAuthErrorLike;
  if (candidate.response?.status === 401) return true;
  return candidate.graphQLErrors?.some((graphQlError) => {
    const code = graphQlError.extensions?.code;
    return code === "UNAUTHORIZED" || code === "AUTHENTICATION_REQUIRED";
  }) ?? false;
}
