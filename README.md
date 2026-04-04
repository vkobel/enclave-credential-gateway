# CoCo Credential Gateway

> A TEE-hardened universal API credential proxy for AI agents, with semantic policy enforcement.

**Status:** Concept / Early Spec  
**Author:** Vinclaw  
**Date:** April 2026

---

## Problem

AI agents need credentials to be useful — GitHub tokens, LLM API keys, payment APIs, communication services. Today, these credentials are either:

1. **Baked into the agent's environment** — visible to any compromised process, logged by accident, leaked through tool calls
2. **Injected at runtime** — still in plaintext in memory, still exfiltrable

The threat is not hypothetical. A compromised dependency, a malicious tool call, a supply chain attack, or simple misconfiguration can silently exfiltrate every secret the agent holds. The agent is the weakest link precisely because it is the most active part of the system.

**Agents run on untrusted platforms.** Most production agent deployments are not in TEEs: laptops, cloud VMs, CI runners, serverless functions, container orchestrators. Hardware attestation of the agent itself is rarely practical or possible. This is the norm, not the exception.

Existing partial solutions:
- **Secret managers** (Vault, AWS Secrets Manager) — protect secrets at rest, but hand them to the agent in plaintext at runtime
- **LLM gateways** (exe.dev, OpenRouter) — abstract LLM API keys from agents, but scoped to LLM calls only, with no attestation or semantic policy
- **TEE-based key management** — protects secrets inside an enclave, but doesn't mediate runtime agent calls

None of these address the full problem: **an agent that can make arbitrary API calls with injected credentials can still do harm**, even without ever seeing the raw key.

---

## Solution

A **remotely accessible credential gateway** running inside a Confidential VM (CVM), exposed as an HTTPS endpoint:

1. **Universal** — covers any HTTP API, not just LLM providers
2. **TEE-hardened** — credentials sealed to hardware state, never leave the enclave boundary
3. **Attestation-verified gateway** — operators and users verify the gateway's hardware attestation before trusting it with credentials; the TLS certificate is cryptographically bound to the enclave
4. **Identity-gated agents** — agents authenticate via API keys or mTLS client certificates, not hardware attestation
5. **Semantically enforced** — per-credential, per-operation, per-agent policies enforced inside the enclave

The agent never holds credentials. It makes calls to the gateway as if it were the upstream API. The gateway enforces policy, injects credentials, and forwards. The raw secret never crosses the enclave boundary — even if the agent is fully compromised.

---

## Architecture

```
┌────────────────────────────────────────────────────────────────────┐
│                     Agents (any platform)                          │
│                                                                    │
│   Laptop          Cloud VM          CI Runner         Container    │
│     │                │                  │                 │        │
│     └────────────────┴──────────────────┴─────────────────┘        │
│                              │                                      │
│                              │ HTTPS + API key / mTLS client cert  │
│                              ▼                                      │
└────────────────────────────────────────────────────────────────────┘
                               │
                               │  TLS terminates inside enclave
                               │  (cert bound to attestation report)
                               ▼
┌────────────────────────────────────────────────────────────────────┐
│              Gateway CVM (Confidential Enclave)                    │
│                                                                    │
│  ┌────────────────────┐                                            │
│  │ Attested TLS       │  ◄── keypair generated inside enclave     │
│  │ Endpoint           │      pubkey in attestation report          │
│  │                    │      TLS cert = proof of enclave binding   │
│  └─────────┬──────────┘                                            │
│            │                                                       │
│            ▼                                                       │
│  ┌────────────────────┐  ┌──────────────┐  ┌────────────────────┐ │
│  │ Agent Identity     │  │ Policy Engine│  │  Credential Vault  │ │
│  │ Gate               │  │              │  │                    │ │
│  │                    │  │ per-provider │  │ sealed to PCR/MRTD │ │
│  │ API key / mTLS     │  │ per-op rules │  │ never leaves CVM   │ │
│  │ → agent identity   │  │ per-agent    │  │                    │ │
│  │ → policy scope     │  │ scoping      │  │                    │ │
│  └──────────┬─────────┘  └──────┬───────┘  └────────┬───────────┘ │
│             │                   │                    │             │
│             └───────────────────┼────────────────────┘             │
│                                 │                                  │
│                          ┌──────▼───────┐                          │
│                          │  Forwarder   │  ◄── injects credential │
│                          │              │      into outbound req   │
│                          └──────┬───────┘                          │
│                                 │                                  │
│                    ┌────────────▼────────────┐                     │
│                    │    Audit Log (sealed)   │                     │
│                    └─────────────────────────┘                     │
│                                                                    │
└────────────────────────────────────────────────────────────────────┘
                               │
                               │ TLS to upstream
                               ▼
              api.github.com / api.openai.com / etc.
```

---

## Components

### 1. Attested TLS Endpoint

The gateway exposes an HTTPS endpoint where TLS termination happens **inside the enclave**:

- The enclave generates its own TLS keypair at startup — the private key never exists outside the CVM
- The public key is included in the attestation report (`reportdata` field)
- The gateway's TLS certificate can be verified against the attestation report
- Anyone connecting can confirm: "this TLS connection terminates inside the real enclave"

This provides **attestation-linked TLS**: the security of the connection is bound to the hardware root of trust, not just a CA signature.

### 2. Agent Identity Gate

Agents authenticate to the gateway using identity-based credentials (not hardware attestation):

- **Per-agent API key** — bearer token in the `Authorization` header, simple to issue and revoke
- **mTLS client certificate** — stronger binding, cert issued per agent identity by the operator

Each agent identity maps to a policy scope:
- Agent A (CI bot) → can call GitHub APIs, read-only
- Agent B (coding assistant) → can call GitHub APIs (read/write) + OpenAI
- Agent C (support bot) → can call Zendesk APIs only

Revocation is immediate: delete the API key or revoke the client cert.

### 3. Credential Vault

- Credentials injected once at setup time via an attested operator channel
- Stored encrypted, sealed to the enclave's hardware measurement (MRTD for TDX, MEASUREMENT for SEV-SNP)
- Unsealed only inside the running enclave; never written in plaintext anywhere
- Optionally backed by a remote KBS (Key Broker Service) for rotation support

### 4. Policy Engine

The policy engine enforces per-credential, per-operation, per-agent rules:

```yaml
credential: github-org
provider: github
base_url: https://api.github.com

agents:
  ci-bot:
    rules:
      - allow:
          methods: [GET]
          path_pattern: "/repos/**"
      - deny:
          methods: ["*"]
          path_pattern: "/**"

  coding-assistant:
    rules:
      - allow:
          methods: [GET, POST, PATCH]
          path_pattern: "/repos/**"
      - allow:
          methods: [POST]
          path_pattern: "/repos/*/issues"
      - deny:
          methods: [DELETE]
          path_pattern: "/**"
      - deny:
          path_pattern: "/repos/*/hooks/**"   # no webhook manipulation
```

Rules are evaluated in order, first match wins. Denied requests are logged and rejected with a 403 — the upstream API is never contacted.

**Semantic extensions (future):**
- Content inspection: reject requests whose body matches patterns (e.g., exfil via gist content)
- Rate limiting per agent identity
- Time-of-day restrictions
- Cross-provider correlation (agent may not POST to GitHub and send email within the same 60s window)

### 5. Forwarder

- Rewrites the inbound agent request: strips gateway path prefix, injects `Authorization` header with the real credential
- Forwards over TLS to the upstream provider
- Streams response back to agent
- Records request metadata (timestamp, agent identity, method, path, response code, bytes) to audit log

### 6. Audit Log

- Append-only log sealed inside the enclave
- Exportable to operator via attested channel (signed by enclave key, verifiable externally)
- Never contains credential values — only call metadata and agent identity
- Enables post-incident forensics on agent behaviour

---

## Gateway Attestation

The gateway's value depends on operators and users being able to verify it before trusting it with credentials.

### Verification Flow

1. **Fetch attestation report** — the gateway exposes `/.well-known/attestation` returning its current TDX/SEV-SNP quote
2. **Verify hardware signature** — confirm the quote is signed by the TEE's hardware root of trust (Intel TDX or AMD SEV-SNP)
3. **Check measurements** — verify MRTD/PCR values match the expected (reproducibly built) gateway binary
4. **Bind TLS to attestation** — the attestation report's `reportdata` contains the hash of the gateway's TLS public key; verify the cert you received matches

### What This Proves

When verification succeeds, you know:
- The gateway binary running is exactly the one you expect (R4: reproducible build)
- The TLS connection terminates inside that enclave (not a MITM)
- Credentials injected into this gateway are sealed to hardware state and cannot be extracted
- Policy enforcement happens inside the enclave boundary

### Trust Model

The **gateway** is the attested party. Operators verify it before injecting credentials. End users can verify it before trusting that their API calls are protected.

Agents are **not** attested — they are identity-authenticated. The gateway cannot prove the agent binary is trustworthy. What it guarantees instead:
- **Credentials never leave the enclave** — even a fully compromised agent cannot extract the raw API key
- **Policy is enforced in hardware** — the agent cannot bypass restrictions even with full host access
- **Every call is audited** — sealed, tamper-evident log of what every agent identity did

---

## Gateway API (Agent-facing)

The agent hits the gateway exactly as it would hit the upstream API, with a path prefix:

```
https://gateway.example.com/gateway/{provider}/{upstream_path}
```

(The base URL is configurable per deployment — could be a public domain, a Tailscale hostname, or a WireGuard-internal address.)

### Authentication

Every request must include agent identity:

```bash
# API key authentication (bearer token)
curl https://gateway.example.com/gateway/github/user/repos \
  -H "Authorization: Bearer <agent-api-key>"

# mTLS authentication (client cert)
curl https://gateway.example.com/gateway/github/user/repos \
  --cert agent.crt --key agent.key
```

### Examples

```bash
# GitHub — list repos
curl https://gateway.example.com/gateway/github/user/repos \
  -H "Authorization: Bearer $GATEWAY_API_KEY"

# OpenAI — chat completion
curl https://gateway.example.com/gateway/openai/v1/chat/completions \
  -H "Authorization: Bearer $GATEWAY_API_KEY" \
  -H "content-type: application/json" \
  -d '{"model": "gpt-4o", "messages": [...]}'

# ElevenLabs — TTS
curl https://gateway.example.com/gateway/elevenlabs/v1/text-to-speech/{voice_id} \
  -H "Authorization: Bearer $GATEWAY_API_KEY" \
  -H "content-type: application/json" \
  -d '{"text": "...", "model_id": "eleven_multilingual_v2"}'
```

No upstream API keys in the request. The gateway injects them. The agent's code is credential-free by design.

---

## Setup Flow

```
Operator                Gateway CVM               KBS (optional)
   │                        │                          │
   │── fetch attestation ──►│                          │
   │◄─ quote + TLS pubkey ──│                          │
   │── verify quote ────────│                          │
   │── verify TLS binding ──│                          │
   │                        │                          │
   │── inject credentials ─►│ (over attested TLS)     │
   │                        │── seal to MRTD ──────────│
   │                        │── store encrypted ───────│
   │                        │                          │
   │── deploy policy docs ─►│                          │
   │                        │── load + compile rules ──│
   │                        │                          │
   │── issue agent API key ►│                          │
   │   (or sign agent cert) │── store agent identity ──│
   │                        │   + policy scope         │
   │                        │                          │
   [Gateway ready for agent calls]
```

### Agent Registration

Each agent identity is:
- A unique identifier (e.g., `ci-bot-prod`, `coding-assistant-alice`)
- An authentication credential (API key or client cert)
- A policy scope (which credentials it can use, which operations are allowed)

Operators issue agent credentials via the attested channel, scoped to the minimum necessary policy set.

---

## CoCo Vector

Target deployment on bare-metal TDX or SEV-SNP (A3):

```
A3 | R4 | B2 | K4
```

- **A3** — direct silicon root of trust, no CSP paravisor
- **R4** — gateway built reproducibly (Nix + Rust), anyone can verify binary matches source
- **B2** — **gateway's TLS public key** hashed into attestation `reportdata`; verifier confirms TLS connection terminates inside the enclave
- **K4** — credential release gated on exact gateway MRTD + operator verification of attestation

Note: B2 applies to the **gateway**, not the agent. Agents are not in TEEs and cannot be hardware-attested. The binding is between the TLS endpoint and the enclave, not between the agent and the enclave.

Cloud deployment (e.g., GCP TDX) is also viable at `A3[GCP TDX] | R4 | B2 | K4` with explicit CSP trust declared.

---

## Implementation Plan

### Phase 1 — Proof of Concept
- [ ] Rust HTTPS server with TLS termination inside enclave
- [ ] TLS keypair generated at startup, pubkey recorded for attestation binding
- [ ] Single credential (GitHub token), hardcoded in config
- [ ] Basic path-prefix routing and credential injection
- [ ] API key authentication (simple bearer token validation)
- [ ] Verify end-to-end: agent calls gateway over HTTPS, GitHub API responds

### Phase 2 — Policy Engine
- [ ] YAML policy document format + parser
- [ ] Per-agent policy scoping
- [ ] Rule evaluator (method + path pattern matching)
- [ ] Deny logging
- [ ] Unit test suite for policy evaluation

### Phase 3 — Credential Vault
- [ ] Encrypted credential store
- [ ] Seal/unseal to TEE measurement (MRTD/PCR)
- [ ] Attested operator injection channel
- [ ] Credential rotation support

### Phase 4 — Attestation Endpoint
- [ ] `/.well-known/attestation` endpoint returning TDX/SEV-SNP quote
- [ ] TLS pubkey hash included in quote's `reportdata`
- [ ] Verification tooling for operators
- [ ] Documentation for third-party verification

### Phase 5 — Agent Identity Management
- [ ] API key issuance, storage, revocation
- [ ] mTLS client cert support (CA inside enclave)
- [ ] Agent identity ↔ policy scope binding
- [ ] Rate limiting per agent identity

### Phase 6 — Audit + Hardening
- [ ] Sealed append-only audit log with agent identity
- [ ] Operator export via attested channel
- [ ] Network egress lockdown (gateway only routes to allowlisted upstream domains)
- [ ] Reproducible Nix build

---

## Tech Stack

- **Language:** Rust (memory safety, no GC pauses, excellent async HTTP with `hyper`/`axum`)
- **Build:** Nix (reproducible, R4-capable)
- **TEE:** Intel TDX (primary), AMD SEV-SNP (secondary)
- **Attestation:** `tdx-attest` crate / `sev` crate
- **TLS:** `rustls` with keypair generated inside enclave at startup
- **Policy:** YAML config, compiled to rule tree at startup
- **Crypto:** `ring` or `aws-lc-rs` for sealing primitives

---

## Non-Goals (v1)

- Hardware attestation of agents (agents are not in TEEs)
- GUI / management console
- Support for non-HTTP protocols (gRPC, SSH)
- Federation between multiple gateway instances

---

## Code References & Reuse

### [`vkobel/enclave-tls-api-fetcher`](https://github.com/vkobel/enclave-tls-api-fetcher) - Primary reference

A Nitro Enclave TLS fetcher with attestation, written in Rust. Large portions are directly reusable or adaptable:

| Crate / Module | Reuse in Gateway | Notes |
|---|---|---|
| `crates/enclave/src/tls/` | TLS termination inside enclave | Adapt for inbound TLS (server), reuse outbound TLS to upstream |
| `crates/enclave/src/vsock/` | Enclave↔host communication channel | Swap for TCP if not using vsock |
| `crates/common/src/hpke.rs` | Attested operator injection channel | HPKE for credential bootstrap — reuse directly |
| `crates/common/src/protocol.rs` | Request/response framing | Adapt to gateway API surface |
| `crates/attestation-verifier/` | Attestation endpoint (Phase 4) | PCR/quote verification library — reuse, swap NSM for TDX/SEV-SNP |
| `crates/host-proxy/` | Agent-facing HTTPS API | Adapt: add API key/mTLS auth layer |

**Key delta from this codebase:**
- Add inbound TLS termination (HTTPS server inside enclave)
- Replace AWS Nitro NSM attestation with Intel TDX / AMD SEV-SNP
- Add Agent Identity Gate (API key + mTLS authentication)
- Add Policy Engine (YAML rules, per-agent scoping)
- Add Credential Vault (sealed encrypted store)

**Estimated savings:** Phases 1 and 4 are significantly de-risked by this reference (~2–3 weeks of work reused).

---

## Related Work

- **exe.dev LLM Gateway** — inspiration for the proxy pattern; scoped to LLM providers, no attestation, no policy
- **Phala Network tappd** — TEE-based secret management for Web3 workloads
- **Practical CoCo Framework** — scoring framework for evaluating this deployment's verifiability posture

---

*Spec draft — April 2026*
