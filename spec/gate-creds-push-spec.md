# Spec: Runtime Service Token Registry

## Goal

Register and rotate named upstream API keys at runtime via the admin API, without redeployment. Credentials live in enclave RAM only. Env vars remain as fallback. No PSK — `GATE_ADMIN_TOKEN` is a vanilla Locksmith-provisioned bearer token verified constant-time per request.

---

## Security model

Locksmith provisions `GATE_ADMIN_TOKEN` at enclave boot; the host never sees it. Admin CLI commands are sent over the steve E2E-encrypted channel (`e2e = true` in config, requires `[attestation]` PCR values). Normal proxy traffic flows on plain HTTP. Credential values are stored in `Zeroizing<String>`; they are never logged, returned in API responses, or printed by the CLI.

---

## API

| Method | Path | Auth | Body | Response |
|---|---|---|---|---|
| `POST` | `/admin/creds` | `Bearer GATE_ADMIN_TOKEN` | `{"name","service","value"}` | 204 / 400 / 401 |
| `GET` | `/admin/creds` | `Bearer GATE_ADMIN_TOKEN` | — | 200 `{"creds":[{"name","service"}]}` |
| `DELETE` | `/admin/creds/{name}` | `Bearer GATE_ADMIN_TOKEN` | — | 204 (idempotent) |

`POST` is an upsert (rotation). Validation: `name` must match `[A-Za-z0-9_-]`, 1–128 chars; `service` must be a known route (`openai`, `anthropic`, `github`).

`POST /admin/tokens` accepts an optional `"creds": {"<route>": "<cred_name>"}` field to bind named creds to the token. Returns 400 if the cred name is not in the store, its service does not match the route, or any `creds` key names a route not included in the token's scope.

---

## CLI commands

```bash
gate admin creds register <service> <value> [--name <name>]   # POST /admin/creds
gate admin creds ls                                            # GET  /admin/creds
gate admin creds rm <name>                                     # DELETE /admin/creds/{name}
gate admin token create --name <n> --scope <routes> --cred route=cred_name
```

Admin commands go through the steve-encrypted channel by default.

---

## Credential resolution order (per request, per route)

1. Phantom token has a `creds` binding for this route → use that named cred from the store (missing or service mismatch → 503, no fall-through).
2. Cred store contains a cred named the same as the route (e.g. `"openai"`) with matching service → use it.
3. Env var (`OPENAI_API_KEY`, `ANTHROPIC_API_KEY`, `GITHUB_TOKEN`).

If all three are absent → 503.

---

## Config

```toml
# ~/.config/gate/config.toml
gateway_url = "https://gw.example.com"
admin_token = "..."
e2e = true   # default; routes admin commands through steve

[attestation]
pcr0 = "..."
pcr1 = "..."
pcr2 = "..."
# base_url = "..."   # optional; defaults to gateway_url
```

Set `e2e = false` for plain HTTP (local dev, no attestation required).

---

## Acceptance criteria

- `POST /admin/creds` with wrong admin token → 401.
- Register `{"name":"openai","service":"openai","value":"sk-..."}` → 204.
- `GET /admin/creds` → 200; body contains `"openai"`; body never contains the value.
- Unknown service → 400; invalid name chars → 400.
- Proxied request to `/openai/v1/models` with registered bogus cred does not return 503 (cred is picked up).
- `DELETE /admin/creds/openai` → 204; cred no longer listed; proxied request reverts (503 if no env key, 200 if env key present).
- Second `DELETE` → 204 (idempotent).
- `POST /admin/tokens` with `"creds":{"github":"gh-e2e"}` (cred registered, service matches) → 200/201.
- `POST /admin/tokens` with unknown cred name → 400.
- `cargo fmt --check` and `cargo test --workspace` pass.
- `./scripts/test-e2e.sh` passes with the admin creds section.
