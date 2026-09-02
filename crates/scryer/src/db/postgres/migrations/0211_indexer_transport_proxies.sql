-- PostgreSQL twin of migrations/0211_indexer_transport_proxies.sql.
-- `protocol` becomes nullable because transport proxies (http, socks5) speak
-- no challenge-solver protocol, and the proxy gains encrypted credentials plus
-- the SOCKS5 remote-DNS (`socks5h`) flag.
ALTER TABLE indexer_proxy_configs
    ALTER COLUMN protocol DROP NOT NULL;

ALTER TABLE indexer_proxy_configs
    ADD COLUMN IF NOT EXISTS username_encrypted text;

ALTER TABLE indexer_proxy_configs
    ADD COLUMN IF NOT EXISTS password_encrypted text;

ALTER TABLE indexer_proxy_configs
    ADD COLUMN IF NOT EXISTS remote_dns boolean DEFAULT false NOT NULL;
