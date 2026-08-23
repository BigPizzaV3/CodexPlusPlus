# Codex++ Share

Cloudflare Pages + Workers KV deployment for encrypted session sharing.

## Cloudflare bindings

- Pages project: `codexpp-share`
- KV binding: `SHARES`
- Custom domain: `share.codexpp.cc`

The browser encrypts content with AES-256-GCM. The encryption key stays in the
URL fragment and is not sent to the server. KV records expire after 1, 7, or 30
days and can be revoked with a separate delete token stored in the creator's
browser.

The API also allows cross-origin requests from the Codex desktop renderer so
the injected session-share button can create links without sending plaintext.

## Limits

- Maximum plaintext length: 900,000 characters
- Maximum encrypted payload: approximately 1 MiB
- Allowed expiration: 1, 7, or 30 days
