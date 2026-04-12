# CoCo Credential Gateway

A confidential credential proxy for AI agents. Agents authenticate with a phantom token; the gateway holds and injects the real upstream credentials inside a hardware TEE — the raw secrets never touch the agent's host.

Built on [`nono-proxy`](./nono) (phantom token pattern + reverse proxy core), promoted into a remotely accessible, hardware-attested network service.

---

## What It Does

- Agents point their `BASE_URL` at the CVM gateway instead of directly at OpenAI, Anthropic, GitHub, etc.
- The gateway validates the phantom token, strips it, injects the real API key from in-enclave secrets, and forwards the request
- A `GET /attest` endpoint returns a raw TDX attestation quote so operators can verify the binary running inside the enclave
- Real credentials never leave the enclave; if an agent is compromised, rotate the phantom token — the upstream keys stay untouched

---

## Architecture

```text
┌─────────────────────────────────────────────────────────┐
│                     Agent (any platform)                │
│  config: BASE_URL = https://<cvm-host>/openai           │
│          PHANTOM_TOKEN = <64-char hex token>            │
└────────────────────────┬────────────────────────────────┘
                         │ HTTPS + Proxy-Authorization: Bearer <phantom-token>
                         ▼
              [ Phala Edge — TLS Termination ]
                         │ HTTP (internal CVM network)
                         ▼
┌─────────────────────────────────────────────────────────┐
│               Phala Cloud TDX CVM                       │
│                                                         │
│  ┌───────────────────────────────────────────────────┐  │
│  │              coco-gateway (Docker container)      │  │
│  │                                                   │  │
│  │  Phantom Token Validator (constant-time)          │  │
│  │       │                                           │  │
│  │  Route Dispatcher  /openai/ /anthropic/ /github/  │  │
│  │       │                                           │  │
│  │  Credential Injector (strips phantom, injects     │  │
│  │  real key from env)                               │  │
│  │       │                                           │  │
│  │  GET /attest → raw TDX QuoteV4                    │  │
│  │                                                   │  │
│  │  Secrets (Phala KMS → env vars inside enclave):   │  │
│  │    COCO_PHANTOM_TOKEN, OPENAI_API_KEY, ...        │  │
│  └───────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────┘
                         │ TLS outbound (rustls)
                         ▼
          api.openai.com / api.anthropic.com / api.github.com
```

---

## Milestones

### Phase 1 — POC (current)

Proves the end-to-end flow: agent → CVM → upstream, real key never leaving the enclave.

Delivered in two steps:

**Path B — Standalone gateway (ship first)**
A new `coco-gateway` Rust binary that uses `nono-proxy` as a Cargo library dependency. Composes its individual modules (`RouteStore`, `reverse`, `credential`, `token::constant_time_eq`) inside a custom Axum server — does not use `nono_proxy::start()` since that forces ephemeral token generation incompatible with the remote pre-shared token model.
- Binds to `0.0.0.0:8080`, deployed on Phala Cloud TDX via Docker Compose
- Phantom token loaded from `COCO_PHANTOM_TOKEN` env var (Phala encrypted secrets); agents authenticate via `Proxy-Authorization: Bearer <token>`
- Routes `/openai/`, `/anthropic/`, `/github/` path prefixes to their respective upstreams; strips phantom token, injects real credential from env
- `GET /attest` returns raw TDX DCAP QuoteV4 via Phala's `tappd` sidecar
- **Known gap:** agents route through CoCo voluntarily via `BASE_URL` — no kernel-level egress enforcement. A compromised agent can bypass the gateway. Mitigate with cloud egress firewall rules; Path C closes this properly.

**Path C — nono fork with client-side attestation (target architecture)**
Fork nono into a workspace that includes `coco-gateway` as a crate alongside the existing CLI. The `nono` CLI gains a `--coco <url>` flag that:
1. Fetches and verifies the CVM's TDX attestation quote before anything else
2. Spawns the sandboxed child process with `NetworkMode::ProxyOnly` pointing at the remote CVM

This restores kernel-level Landlock egress enforcement and bakes attestation verification into the client, closing the two main POC gaps.

```
nono fork workspace:
  crates/
    nono-cli/      ← add --coco flag + attestation verification
    nono-proxy/    ← unchanged
    nono/          ← unchanged
    coco-gateway/  ← new: CVM binary (promoted from Path B)
```

See [`openspec/changes/poc-v1-coco-creds-gateway/`](./openspec/changes/poc-v1-coco-creds-gateway/) for full spec and tasks.

### Phase 2 — Policy + Identity
OpenAI-compatible endpoints, identity-bearing tokens or mTLS, deterministic method/path policy enforcement, per-agent token budget tracking.

### Phase 3 — Portable Encrypted Vaults
Encrypted vault file (`coco-vault.enc`) sealed to the TDX measurement (`MRTD`). Decrypts only if the exact binary version is running — wrong binary, wrong key derivation, vault stays locked. Replaces Phala secret injection as the credential distribution primitive and makes the gateway platform-portable.

### Phase 4 — Compatibility Adapters
Optional wrappers for tools that cannot use provider-shaped HTTP endpoints: env var injection, HTTP proxy mode, config file generation. Secondary to the core proxy thesis.

### Phase 5 — Audit & Attestation
`/.well-known/attestation` endpoint, TLS public key bound into the attestation quote (attested TLS), reproducible Nix builds, tamper-evident audit log.

---

## References

- [`nono/`](./nono) — nono-proxy source (phantom token pattern, route store, credential injection, host filtering)
- [`openspec/`](./openspec) — specs, design docs, and task lists
- [Phala Cloud](https://phala.network) — TDX CVM deployment platform
- [`lunal-dev/attestation-rs`](https://github.com/lunal-dev/attestation-rs) — TDX quote generation library
- [`vkobel/enclave-tls-api-fetcher`](https://github.com/vkobel/enclave-tls-api-fetcher) — TLS-in-enclave reference implementation

---

_April 2026_
