-- A challenge solver Scryer reaches through one of its transport proxies or
-- tunnels. NULL keeps the solver reached directly, as before.
ALTER TABLE proxy_configs ADD COLUMN IF NOT EXISTS reach_through_proxy_config_id text;

CREATE INDEX IF NOT EXISTS idx_proxy_configs_reach_through
ON proxy_configs(reach_through_proxy_config_id);
