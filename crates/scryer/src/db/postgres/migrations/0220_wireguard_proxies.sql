-- PostgreSQL twin of migrations/0214_wireguard_proxies.sql.
--
-- Seven added columns and nothing else, so both engines express this the same
-- way. See the SQLite file for why `peer_public_key` and `tunnel_public_key`
-- are stored in the clear while `preshared_key_encrypted` is not, and why the
-- address and DNS lists are text rather than a child table.
ALTER TABLE proxy_configs
    ADD COLUMN IF NOT EXISTS peer_public_key text;

ALTER TABLE proxy_configs
    ADD COLUMN IF NOT EXISTS preshared_key_encrypted text;

ALTER TABLE proxy_configs
    ADD COLUMN IF NOT EXISTS tunnel_public_key text;

ALTER TABLE proxy_configs
    ADD COLUMN IF NOT EXISTS tunnel_addresses text;

ALTER TABLE proxy_configs
    ADD COLUMN IF NOT EXISTS tunnel_dns_servers text;

ALTER TABLE proxy_configs
    ADD COLUMN IF NOT EXISTS tunnel_mtu integer;

ALTER TABLE proxy_configs
    ADD COLUMN IF NOT EXISTS tunnel_keepalive_seconds integer;
