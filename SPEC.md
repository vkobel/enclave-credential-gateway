# CoCo Credential Gateway — POC Specification

> **Scope:** This document specifies only the Proof of Concept (POC). It deliberately excludes the policy engine, multi-tenancy, and portable encrypted vaults. Sections marked _[Next Step]_ document the architectural path forward without implementing it.

---

## Goal

Demonstrate that `nono-proxy`'s credential injection and phantom token pattern can run inside a Phala Cloud TDX CVM and be securely accessed by remote agents — with the real upstream credentials never leaving the enclave.

**What success looks like for the POC:**

- An agent on a laptop sends an OpenAI-shaped request to the CVM gateway using only a phantom token
- The gateway validates the token, injects the real OpenAI key (loaded from Phala secrets), forwards the request, and returns the response
- The real API key is never visible to the agent or the agent's host
- A raw TDX attestation quote is available at `GET /attest` for operator verification

---

## Platform: Phala Cloud (TDX CVM)

Phala Cloud runs Docker Compose applications inside Intel TDX Confidential VMs on their worker network. It provides:

- **TDX hardware isolation:** Guest memory is encrypted and inaccessible to the host/hypervisor
- **HTTPS ingress:** Phala terminates TLS at their edge and forwards traffic to the container over an internal network
- **Secrets injection:** The operator encrypts secrets client-side (X25519 + AES-256-GCM); only the CVM's TEE can decrypt them at boot; they are injected as environment variables inside the container

> ⚠️ **TLS ingress note (POC limitation):** Phala's TLS terminates at their ingress infrastructure, outside the enclave. This means the full "attested TLS" property — where the TLS public key is cryptographically bound to the TDX attestation quote — is not achieved in this POC. Traffic between Phala's edge and the container is plaintext on the internal CVM network. This is acceptable for a POC focused on credential injection, but must be evaluated before production use. See _[Next Step — In-Enclave TLS]_ below.

---

## Architecture

```text
┌─────────────────────────────────────────────────────────┐
│                     Agent (any platform)                │
│  config: BASE_URL = https://<cvm-host>/openai           │
│          PHANTOM_TOKEN = <64-char hex token>            │
└────────────────────────────┬────────────────────────────┘
                             │ HTTPS + Proxy-Authorization: Bearer <phantom-token>
                             ▼
                  [ Phala Edge — TLS Termination ]  ← ⚠️ outside enclave (POC)
                             │ HTTP (internal CVM network)
                             ▼
┌─────────────────────────────────────────────────────────┐
│               Phala Cloud TDX CVM                       │
│                                                         │
│  ┌───────────────────────────────────────────────────┐  │
│  │              coco-gateway (Docker container)      │  │
│  │                                                   │  │
│  │  ┌─────────────────────────────────────────────┐  │  │
│  │  │ Phantom Token Validator                     │  │  │
│  │  │  - Validates Proxy-Authorization header     │  │  │
│  │  │  - Constant-time comparison (no timing leak)│  │  │
│  │  └──────────────────┬──────────────────────────┘  │  │
│  │                     │ authenticated                │  │
│  │  ┌──────────────────▼──────────────────────────┐  │  │
│  │  │ Route Dispatcher (nono-proxy RouteStore)    │  │  │
│  │  │  /openai/... → api.openai.com               │  │  │
│  │  │  /anthropic/... → api.anthropic.com         │  │  │
│  │  │  /github/... → api.github.com               │  │  │
│  │  └──────────────────┬──────────────────────────┘  │  │
│  │                     │                             │  │
│  │  ┌──────────────────▼──────────────────────────┐  │  │
│  │  │ Credential Injector (nono-proxy ReverseProxy│  │  │
│  │  │  Strips phantom token, injects real key:    │  │  │
│  │  │  Authorization: Bearer <real-key-from-env>  │  │  │
│  │  └──────────────────┬──────────────────────────┘  │  │
│  │                     │                             │  │
│  │  ┌──────────────────▼──────────────────────────┐  │  │
│  │  │ GET /attest → raw TDX quote (attestation-rs)│  │  │
│  │  └─────────────────────────────────────────────┘  │  │
│  │                                                   │  │
│  │  Secrets (in-enclave env vars, from Phala KMS):  │  │
│  │    COCO_PHANTOM_TOKEN, OPENAI_API_KEY, ...        │  │
│  └───────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────┘
                             │ TLS (outbound, rustls)
                             ▼
              api.openai.com / api.github.com / ...
```

---

## The Phantom Token Pattern (Remote)

This is the core security primitive of the POC. Understanding how it differs from nono's local model is important.

### How it works in nono (local)

nono-proxy generates a random 32-byte session token at startup and injects it into child processes via the `NONO_PROXY_TOKEN` environment variable. The child authenticates to the proxy by sending `Proxy-Authorization: Bearer <token>` with every request. The proxy validates with constant-time comparison, then strips the token and injects the real upstream credential before forwarding.

The session token is *ephemeral* (regenerated each run) and *local* (delivered via env var to a child on the same machine).

### Adaptation for CoCo (remote)

The proxy is now a long-running network service. The phantom token must be:

1. **Pre-shared** — not ephemeral. The operator defines a strong random phantom token and stores it in Phala secrets. The gateway loads it on startup. Agents are configured with the same token out-of-band (e.g., written into their config files or injected via their own secret manager).
2. **Transmitted over the network** — the agent includes `Proxy-Authorization: Bearer <phantom-token>` on every request. Over Phala's HTTPS this is protected in transit.

**Request flow:**

```
Agent:
  POST https://<cvm-host>/openai/v1/chat/completions
  Proxy-Authorization: Bearer <phantom-token>
  Content-Type: application/json
  { "model": "gpt-4o", "messages": [...] }

Gateway (inside CVM):
  1. Validates Proxy-Authorization header (constant-time compare vs COCO_PHANTOM_TOKEN)
  2. Resolves route: /openai/ → api.openai.com
  3. Strips Proxy-Authorization header
  4. Injects: Authorization: Bearer <OPENAI_API_KEY from env>
  5. Forwards: POST https://api.openai.com/v1/chat/completions

Upstream:
  Responds normally

Gateway:
  Streams response back to agent
```

**Revocation:** If an agent is compromised, the operator rotates `COCO_PHANTOM_TOKEN` in Phala secrets and redeploys. The real upstream API keys never need rotating — they are never visible to the agent's host.

**Key distinction from API key auth:** A phantom token authenticates the *agent session*, not the upstream service. The real upstream credential is a separate secret the gateway holds. The agent cannot derive, observe, or exfiltrate it.

---

## Components

### 1. nono-proxy Adaptation

`nono-proxy` is currently a localhost-only sidecar (`127.0.0.1:0`). The changes needed to run it as a CVM gateway:

**Changes required:**
- **Bind address:** Change from `127.0.0.1` to `0.0.0.0` (listen on all interfaces)
- **Fixed port:** Replace OS-assigned port with a configured port (e.g., `8080`)
- **Token source:** Load phantom token from `COCO_PHANTOM_TOKEN` env var instead of generating ephemeral token
- **Credential source:** Load upstream credentials from env vars (`OPENAI_API_KEY`, `GITHUB_TOKEN`, etc.) — already supported via nono-proxy's env var backend in `keystore.rs`
- **Route config:** Define routes via config file or env var (e.g., `/openai/` → `api.openai.com`) — already supported via `RouteStore`

**No changes required:**
- Credential injection logic (`reverse.rs`) — reuse as-is
- Token validation logic (`token.rs`) — reuse as-is, including constant-time comparison
- Outbound TLS to upstreams (`tokio-rustls` in `reverse.rs`) — reuse as-is
- Host filtering (`filter.rs`) — reuse as-is

### 2. Phala Secret Injection

Upstream credentials and the phantom token are injected via Phala's encrypted secrets mechanism:

- **Client-side:** Operator encrypts secrets via Phala dashboard, CLI (`phala secrets set`), or SDK. Encryption uses X25519 key exchange + AES-256-GCM; ciphertext is sent to Phala's servers.
- **At boot:** The CVM's TEE decrypts the secrets using a key derived from the TDX measurement. Plaintext values are set as environment variables before Docker Compose services start.
- **Inside the container:** Gateway reads them from env on startup. They exist only in enclave memory.

Secrets required for POC:

| Env var | Description |
|---------|-------------|
| `COCO_PHANTOM_TOKEN` | 64-char hex token agents use to authenticate |
| `OPENAI_API_KEY` | Real OpenAI key (injected by gateway, never sent to agents) |
| `ANTHROPIC_API_KEY` | Real Anthropic key (optional for POC) |
| `GITHUB_TOKEN` | Real GitHub PAT (optional for POC) |

### 3. Attestation Endpoint

The gateway exposes `GET /attest` returning the raw TDX DCAP QuoteV4 in hex or base64. This allows an operator to:

1. Retrieve the quote
2. Verify the TDX certificate chain (PCK → Intermediate → Intel Root CA)
3. Check the `td_attributes` field (bit 0 must be 0 — reject debug mode)
4. Inspect the `MRTD` / `RTMR` values against known-good measurements from a reproducible build

**Library:** [`lunal-dev/attestation-rs`](https://github.com/lunal-dev/attestation-rs) — supports Intel TDX bare-metal and cloud (including Phala/dstack TDX). Provides both attester (generate quote) and verifier (parse/verify) APIs. Use the `attest` feature flag for guest-side quote generation.

**POC implementation:** At startup, generate a quote with a nonce derived from the current timestamp in `report_data`. Serve it at `GET /attest`. No client verification is enforced in the POC — this endpoint is for operator transparency.

> ⚠️ **Debug mode check:** Always verify `td_attributes[0] & 0x01 == 0` before trusting any quote. Debug mode means host can read TD memory — attestation is meaningless.

### 4. Deployment (Docker Compose on Phala Cloud)

The gateway runs as a single Docker container. Minimal `docker-compose.yml`:

```yaml
services:
  coco-gateway:
    image: ghcr.io/<org>/coco-gateway:latest
    ports:
      - "8080:8080"
    environment:
      - COCO_PHANTOM_TOKEN=${COCO_PHANTOM_TOKEN}
      - OPENAI_API_KEY=${OPENAI_API_KEY}
      - GITHUB_TOKEN=${GITHUB_TOKEN}
    restart: unless-stopped
```

Secrets are set via Phala CLI before deployment:
```bash
phala cvms secrets set COCO_PHANTOM_TOKEN="<64-char-hex>"
phala cvms secrets set OPENAI_API_KEY="sk-..."
```

Phala assigns a public HTTPS hostname (e.g., `https://<id>.phala-cvms.com`). Agents point their `BASE_URL` at this hostname with a path prefix.

---

## What is Deliberately Excluded from POC

| Feature | Reason deferred |
|---------|-----------------|
| Policy engine (method/path rules) | Adds complexity without changing the security primitive |
| Multi-tenancy / per-agent identity | Requires identity infrastructure beyond a single phantom token |
| Audit log | Out of scope; no tamper-evidence requirement for POC |
| Reproducible builds | Required for meaningful attestation pinning; deferred to Phase 5 |
| Rate limiting / token budgets | Policy layer, deferred |

---

## Next Steps (Architecture Only — Not Implemented in POC)

### [Next Step A] In-Enclave TLS Termination

The POC uses Phala's edge TLS, which terminates outside the enclave. For full "attested TLS" — where the TLS keypair is cryptographically bound to the TDX quote — TLS must terminate inside the CVM.

**Approach:**

1. At CVM startup, generate an ephemeral TLS keypair inside the enclave using `rustls` with `ring`/`aws-lc-rs` (both use the OS entropy source, which in a TDX CVM draws from hardware RNG)
2. Hash the TLS public key: `SHA-512("coco-tls-pubkey:" || pubkey_der)` → write into `report_data`
3. Generate a TDX quote with this `report_data` binding
4. Expose the quote + public key at `GET /attest` so agents can verify the TLS endpoint they're talking to matches the attested binary
5. Expose a raw TCP port from the CVM (bypassing Phala's HTTPS ingress) and handle TLS entirely within the enclave

**References:**
- [Turnkey: TLS Sessions Within TEEs](https://quorum.tkhq.xyz/posts/tls-sessions-within-tees/) — Rustls + custom Read/Write transport pattern
- [`vkobel/enclave-tls-api-fetcher`](https://github.com/vkobel/enclave-tls-api-fetcher) — TLS patterns reusable; vsock transport is Nitro-specific and does not apply to Phala TDX

**Note:** Phala's networking model must be verified to confirm whether raw TCP ports can be exposed from the CVM directly, bypassing their HTTPS edge. If not, an alternative is to use Phala's TLS while separately publishing the TLS certificate fingerprint alongside the attestation quote, allowing agents to pin the certificate.

### [Next Step B] Secure Credential Injection — Portable Encrypted Vaults

Phala's secret injection (X25519 + AES-256-GCM, decrypted inside TEE) is the right model for a single-platform deployment. For cross-platform portability and operator-controlled credential management, the next step is a **portable encrypted vault**:

**Design sketch:**

1. Operator creates a credential vault file (`coco-vault.enc`) locally, encrypting API keys with a vault passphrase
2. The vault file's encryption key is *wrapped* (HKDF-derived from the TDX measurement `MRTD` + operator passphrase)
3. The vault file is uploaded to the CVM (or baked into the container image as a non-secret artifact — it's encrypted)
4. At boot, the gateway decrypts the vault **only if** the TDX measurement matches the expected value — enforced by deriving the decryption key from `MRTD` via HKDF
5. If the binary has been tampered with (different `MRTD`), the decryption key derivation produces the wrong key; the vault cannot be opened

**Security properties this adds over Phala secrets:**
- Credentials are portable across platforms (not tied to Phala's KMS)
- The vault can only be decrypted by the exact binary version it was sealed for
- Operator can verify the binary before unsealing via remote attestation

**Key derivation pattern:**
```
vault_key = HKDF-SHA256(
    ikm   = operator_passphrase,
    salt  = MRTD_measurement,  // TDX register, locked at boot
    info  = b"coco-vault-v1"
)
```

This is the long-term credential distribution primitive. Phala's secrets serve as the bootstrapping mechanism for the POC.

---

## Implementation Checklist (POC)

- [ ] Fork / adapt `nono-proxy` into a standalone `coco-gateway` binary
  - [ ] Configurable bind address (`0.0.0.0:8080`)
  - [ ] Phantom token loaded from `COCO_PHANTOM_TOKEN` env var (not generated ephemerally)
  - [ ] Routes configured via YAML/TOML or env var (at minimum: `/openai/`, `/anthropic/`, `/github/`)
  - [ ] Credentials loaded from env vars via existing keystore env backend
- [ ] Add `GET /attest` endpoint using `attestation-rs` (lunal-dev)
  - [ ] Detect platform (Phala TDX) via `attestation::detect()`
  - [ ] Generate quote, return as JSON `{ "quote": "<hex>", "platform": "tdx" }`
  - [ ] Assert `td_attributes` debug bit is unset
- [ ] Docker container + `docker-compose.yml`
- [ ] Phala Cloud deployment
  - [ ] Set secrets via Phala CLI
  - [ ] Verify secrets are available as env vars inside the container
  - [ ] Verify `GET /attest` returns a valid TDX quote
- [ ] End-to-end agent test
  - [ ] Configure an OpenAI client to use `BASE_URL=https://<cvm-host>/openai/v1`
  - [ ] Configure `Proxy-Authorization: Bearer <phantom-token>`
  - [ ] Confirm request succeeds and real key is never in agent process memory

---

_April 2026_
