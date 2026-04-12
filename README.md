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
Standalone `coco-gateway` binary running on Phala Cloud TDX. Phantom token auth, multi-provider routing, credential injection from env, raw TDX attestation quote at `GET /attest`. Proves the end-to-end flow: agent → CVM → upstream, real key never leaving the enclave.

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
