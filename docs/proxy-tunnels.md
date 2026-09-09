# Tunnel proxies

Scryer uses the shared `scryer-media/proxy-tunnels` crate at Git tag `v0.1.0`
(`01481be7fca27a7864fe40e9a4b95fcfa9698ce2`). The dependency retains the
`scryer_tunnel` import name. Its HTTP/3 feature uses AWS-LC for QUIC TLS;
Quinn's default crypto features remain disabled.

## HTTP/3 CONNECT

Select **HTTP/3 CONNECT** in Settings → Proxies. Enter an `https://host:port`
endpoint; the default port is 443. This must be an upstream HTTP/3 proxy that
accepts ordinary CONNECT. A general HTTP/3 website is insufficient. UDP must
be able to reach the proxy endpoint.

Optional username/password authentication uses HTTP Basic inside the verified
QUIC TLS connection. Credentials use the existing encrypted proxy storage and
write-only API fields. HTTP/3 profiles do not accept SSH or WireGuard keys.
Proxy certificates must chain to the shared crate's public root store. The
indexer's custom CA bundle applies to the destination TLS connection, not the
outer QUIC proxy connection.

Assign the profile to an indexer or download client using the existing proxy
selector. Destination names travel in CONNECT authority and resolve at the
proxy. The host resolves only the proxy endpoint. An unavailable or rejected
proxy fails the request; it does not fall back to direct networking.

Each saved profile's Test action opens a form below the proxy table, defaulting to
`https://api64.ipify.org?format=json`. Enable the profile, then test to fetch
the URL through its saved route and display its public exit IP, destination
HTTP status, and elapsed time. No assigned indexer is needed. A custom page
that does not return an IP reports its HTTP status without changing the
profile's saved health. Solvers fetch the URL themselves and report their
own exit IP; this does not verify challenge solving or other Scryer traffic.
API callers that omit the test URL retain the existing health-check behavior.
Download clients retain their own connection test.

## Connection reuse and resource limits

All tunnel consumers share one lazy tunnel front per profile revision. HTTP/3
requests multiplex independent CONNECT streams on a reused QUIC session.
SSH and WireGuard likewise reuse their existing provider sessions. Updating
a profile closes the old route before the mutation completes. Front creation
is serialized with settings activation; stale, disabled, and deleted profile
snapshots cannot recreate or replace the current tunnel.

Artifact downloads reuse HTTP clients across requests and redirect hops. The
cache keeps at most 32 profiles, expires entries after five minutes, and
replaces clients when either the profile revision or tunnel front changes.
Redirects remain under the artifact transport's per-hop handling, including
cross-origin credential removal. Async and blocking proxy clients retain at
most four idle connections per host, expiring after 60 seconds.

WireGuard retains the shared crate's 64 KiB TCP buffers for Scryer's API and
artifact workload. Weaver's larger download buffers are not enabled globally.
The tag supplies the shared SSH, SOCKS bridge, and WireGuard performance fixes.

## SSH authentication

SSH uses an Ed25519 private key, including support for encrypted keys and their
passphrases. A username and private key are required to save a profile.
Host-key trust uses an atomic revision-checked database write; failed or
cancelled writes retain the
original pending decision. Only an explicit trust reset permits a new key.

## Local regression coverage

Local HTTP/3 fixtures cover verified TLS, rejected roots and credentials,
ordinary CONNECT encoding, remote destination names, async and blocking
consumers, shared QUIC sessions, and profile revision replacement. SSH-backed
artifact fixtures assert that repeated requests reuse a single channel and
that edits and expiry replace the pool. Storage and editor tests cover
encrypted HTTP/3 credentials, SSH key authentication, and provider-specific
controls.
