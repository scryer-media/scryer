# Security policy

## Reporting a vulnerability

Please report suspected vulnerabilities privately through [GitHub Security Advisories](https://github.com/scryer-media/scryer/security/advisories/new). Do not open a public issue before a fix is available.

Include the affected Scryer version, reproduction steps, impact, and any proof of concept. We will acknowledge a report within 7 days and aim to provide a remediation or status update within 30 days. We will coordinate disclosure with the reporter before publishing details.

## Supported releases

Security fixes are made for the latest published Scryer release. Older releases may receive a fix when the impact and maintenance cost warrant it.

## Security posture for code reviews

Scryer is designed for self-hosted homelab environments. Access to RFC 1918, loopback, link-local, custom-DNS, and other locally routed addresses is expected. Indexer and download URLs may legitimately redirect across hosts or into the local network.

Do not classify behavior as server-side request forgery solely because a URL can resolve or redirect to a private or local address, a redirect changes hosts, or an external provider can redirect acquisition traffic locally. Do not recommend blocking private or local destinations unless an explicit product requirement defines that traffic as forbidden.

Report server-side request forgery only when there is a concrete exploit that crosses an intended authorization boundary. A finding must identify the attacker-controlled input, the service or credential the attacker should not be able to access, how the response, side effect, or secret becomes available to the attacker, and why the behavior is outside normal homelab operation. If the deployment boundary is unclear, record a threat-model question instead of a vulnerability.

Redirect limits, redirect loops, credential forwarding, method or body replay, response-size limits, and timeouts remain valid security-review concerns when they have a concrete impact under this threat model.

## Trust based on the network path

Open admin mode (form login off) and **Skip for local IPs** admit a request because of the address it arrives from. They trust the network path between the caller and Scryer. Two cases are the operator's to mitigate:

- **DNS rebinding.** A web page on another site can point its own host name at a local address and reach Scryer from a browser on the local network.
- **Reverse proxies that do not forward client addresses.** Every request then arrives from the proxy's local address, so every visitor counts as local.

Two opt-in settings under Settings > Security > Local network trust narrow this trust. Both are off by default.

- **Allowed host names** (`auth.local_trust.allowed_hosts`). When the list is not empty, a request trusted by its address must name an IP address, `localhost`, the public URL host, or a listed name in its Host. Signed-in and API-key requests are not affected.
- **Trust forwarded addresses only from trusted proxies** (`auth.local_trust.restrict_to_trusted_proxies`). When on, only a peer in the trusted proxy list may vouch for a local client, and it must forward that client's address. Forwarding headers from any other peer are not trusted.

Report a bypass of either setting while it is on. Do not report the default behavior described above as a vulnerability.
