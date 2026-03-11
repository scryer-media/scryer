/// <reference types="vite/client" />

interface ImportMetaEnv {
  readonly SCRYER_BASE_PATH: string;
  readonly SCRYER_GRAPHQL_URL: string;
  readonly SCRYER_METADATA_GATEWAY_GRAPHQL_URL: string;
}

interface ImportMeta {
  readonly env: ImportMetaEnv;
}
