import { LIST_SUBSCRIPTION_FIELDS } from "./queries";

const ACCOUNT_FIELDS = `
  id provider externalUserId username displayName status errorMessage linkedAt lastUsedAt
  ownedLists { id name kinds sourceType params { key value } }
  statuses { key label kinds sourceType params { key value } }`;

export const myListAccountsQuery = `query MyListAccounts {
  myListAccounts {${ACCOUNT_FIELDS}}
}`;

export const listAccountQuery = `query ListAccount($id: ID!) {
  listAccount(id: $id) {${ACCOUNT_FIELDS}}
}`;

export const myListSubscriptionsQuery = `query MyListSubscriptions {
  myListSubscriptions {${LIST_SUBSCRIPTION_FIELDS}}
}`;

export const startListAccountLinkMutation = `mutation StartListAccountLink($provider: String!, $origin: String!) {
  startListAccountLink(provider: $provider, origin: $origin) {
    sessionId state authorizeUrl authorizationOrigin expiresAt pollRequired
  }
}`;

export const linkListAccountWithKeyMutation = `mutation LinkListAccountWithKey($provider: String!, $key: String!, $origin: String) {
  linkListAccountWithKey(provider: $provider, key: $key, origin: $origin) {
    account {${ACCOUNT_FIELDS}}
    link { sessionId state authorizeUrl authorizationOrigin expiresAt pollRequired }
  }
}`;

export const pollListAccountLinkMutation = `mutation PollListAccountLink($sessionId: ID!) {
  pollListAccountLink(sessionId: $sessionId) {
    status account {${ACCOUNT_FIELDS}}
  }
}`;

export const cancelListAccountLinkMutation = `mutation CancelListAccountLink($sessionId: ID!) {
  cancelListAccountLink(sessionId: $sessionId)
}`;

export const completeListAccountLinkMutation = `mutation CompleteListAccountLink($sessionId: ID!, $state: String!, $provider: String!, $code: String!, $issuer: String) {
  completeListAccountLink(sessionId: $sessionId, state: $state, provider: $provider, code: $code, issuer: $issuer) {${ACCOUNT_FIELDS}}
}`;

export const unlinkListAccountMutation = `mutation UnlinkListAccount($id: ID!) {
  unlinkListAccount(id: $id)
}`;

export const listProviderAppsQuery = `query ListProviderApps {
  listProviderApps { provider clientId redirectUri clientSecretSet enabled }
}`;

/** Only what the Lists page's tab counters need. */
export const listTabCountsQuery = `query ListTabCounts($personal: Boolean!, $exclusions: Boolean!) {
  listSubscriptions { id }
  myListSubscriptions @include(if: $personal) { id }
  listExclusions @include(if: $exclusions) { id }
}`;

export const updateListProviderAppMutation = `mutation UpdateListProviderApp($provider: String!, $clientId: String, $clientSecret: String, $redirectUri: String, $enabled: Boolean!) {
  updateListProviderApp(provider: $provider, clientId: $clientId, clientSecret: $clientSecret, redirectUri: $redirectUri, enabled: $enabled) {
    provider clientId redirectUri clientSecretSet enabled
  }
}`;

export const personalListRouteOptionsQuery = `query PersonalListRouteOptions {
  qualityProfileSettings { profiles { id name } }
  manageableLibraries: libraries(permission: MANAGE_TITLES) {
    id facet name slug isDefault enabled qualityProfileId
    roots { id path isDefault }
  }
  requestableLibraries: libraries(permission: REQUEST) {
    id facet name slug isDefault enabled requestQualityProfileIds requestQualityProfileDefaultId
    roots { id path isDefault }
  }
}`;
