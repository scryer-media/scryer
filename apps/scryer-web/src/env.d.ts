/// <reference types="vite/client" />

declare module "@fontsource-variable/*";
// cronstrue locale modules register themselves on import and ship no types.
declare module "cronstrue/locales/*";

interface ImportMetaEnv {
  readonly SCRYER_BASE_PATH: string;
  readonly SCRYER_GRAPHQL_URL: string;
  readonly SCRYER_METADATA_GATEWAY_GRAPHQL_URL: string;
}

interface ImportMeta {
  readonly env: ImportMetaEnv;
}
