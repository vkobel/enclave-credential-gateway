# CoCo Credential Gateway

A credential proxy for AI agents: agents authenticate with a phantom token, the gateway validates it and injects the real upstream API key, so secrets never touch the agent's host.

Built on [`nono-proxy`](./nono) — promoted into a remotely deployable, hardware-attested service.

---

## Quickstart

**1. Generate a phantom token** — this is the shared secret your agents will use as their API key:

```bash
export COCO_PHANTOM_TOKEN=$(openssl rand -hex 32)
echo $COCO_PHANTOM_TOKEN   # save this, you'll pass it to agents
```

**2. Set your upstream credentials** and start the gateway:

```bash
export OPENAI_API_KEY=sk-...
export ANTHROPIC_API_KEY=sk-ant-...
export GITHUB_TOKEN=ghp_...
export HTTPBIN_TOKEN=any-value   # dummy value works for smoke-testing

docker compose up -d --build
```

Routes are loaded from [`examples/profile.json`](./examples/profile.json) — edit it to add or remove upstreams.

**3. Call any upstream through the gateway** — real keys never leave the gateway:

```bash
# httpbin — good smoke test, no real credential needed
curl http://localhost:8080/httpbin/bearer \
  -H "Proxy-Authorization: Bearer $COCO_PHANTOM_TOKEN"
# → {"authenticated": true, "token": "any-value"}

# OpenAI — agents set OPENAI_BASE_URL=http://localhost:8080/openai/v1
curl -X POST http://localhost:8080/openai/v1/chat/completions \
  -H "Proxy-Authorization: Bearer $COCO_PHANTOM_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"model":"gpt-4o-mini","messages":[{"role":"user","content":"ping"}]}'

# Anthropic
curl -X POST http://localhost:8080/anthropic/v1/messages \
  -H "Proxy-Authorization: Bearer $COCO_PHANTOM_TOKEN" \
  -H "Content-Type: application/json" \
  -H "anthropic-version: 2023-06-01" \
  -d '{"model":"claude-haiku-4-5-20251001","max_tokens":64,"messages":[{"role":"user","content":"ping"}]}'
```

The gateway returns `407` on a missing/wrong phantom token, `404` on an unknown prefix, `503` if the upstream credential env var is absent.

---

## Custom Profiles

Routes are defined in [`examples/profile.json`](./examples/profile.json) and mounted into the container by `docker-compose.yml`. Edit that file to add any upstream:

```json
{
  "routes": {
    "httpbin": {
      "upstream": "https://httpbin.org",
      "credential_env": "HTTPBIN_TOKEN"
    },
    "openai": {
      "upstream": "https://api.openai.com",
      "credential_env": "OPENAI_API_KEY"
    }
  }
}
```

| Field | Required | Default | Description |
|---|---|---|---|
| `upstream` | yes | — | HTTPS upstream base URL |
| `credential_env` | yes | — | Env var holding the real credential |
| `inject_header` | no | `Authorization` | Header name to inject the credential into |
| `credential_format` | no | `Bearer {}` | Format string; `{}` is replaced with the credential |

After editing the profile, restart with `docker compose up -d --build`. Override the profile path with `COCO_PROFILE=/custom/path.json`.

---

## How It Works

```
Agent  ──Proxy-Authorization: Bearer <phantom>──▶  coco-gateway
                                                        │
                                              validate token (constant-time)
                                                        │
                                              match /<prefix>/ → upstream
                                                        │
                                              strip phantom, inject real key
                                                        │
                                                        ▼
                                           api.openai.com (TLS, rustls)
```

**Known gap (POC):** Agents route through the gateway voluntarily via `BASE_URL`. A compromised agent can bypass by connecting directly to the upstream. Mitigate with egress firewall rules. Path C (nono fork + Landlock enforcement) closes this properly.

---

## Configuration

| Env var | Required | Default | Description |
|---|---|---|---|
| `COCO_PHANTOM_TOKEN` | yes | — | Shared secret agents use as their API key |
| `COCO_LISTEN_PORT` | no | `8080` | Port to bind |
| `COCO_PROFILE` | no | `/etc/coco/profile.json` | Profile file path |

---

## Milestones

**Phase 1a (current) — Plain infrastructure POC**
`coco-gateway` binary on any Docker host. Proves the proxy data plane end-to-end.

**Phase 1b — CVM attestation**
Promote to Phala Cloud TDX CVM. Add `GET /attest` (raw TDX QuoteV4 via `tappd` sidecar).

**Phase 2+ — Policy, identity, encrypted vaults, audit**
See [`openspec/`](./openspec) for specs and task lists.

---

## References

- [`nono/`](./nono) — nono-proxy (phantom token pattern, route store, credential injection)
- [`openspec/`](./openspec) — specs, design docs, task lists
- [Phala Cloud](https://phala.network) — TDX CVM deployment platform
