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

Existing partial solutions:
- **Secret managers** (Vault, AWS Secrets Manager) — protect secrets at rest, but hand them to the agent in plaintext at runtime
- **LLM gateways** (exe.dev, OpenRouter) — abstract LLM API keys from agents, but scoped to LLM calls only, with no attestation or semantic policy
- **TEE-based key management** — protects secrets inside an enclave, but doesn't mediate runtime agent calls

None of these address the full problem: **an agent that can make arbitrary API calls with injected credentials can still do harm**, even without ever seeing the raw key.

---

## Solution

A **link-local credential gateway** running inside a Confidential VM (CVM), modelled on the exe.dev LLM gateway pattern (`169.254.x.x`) but:

1. **Universal** — covers any HTTP API, not just LLM providers
2. **TEE-hardened** — credentials sealed to hardware state, never exfiltrated from the enclave boundary
3. **Attestation-gated** — agent must prove its identity before the gateway activates
4. **Semantically enforced** — per-credential, per-operation policies enforced inside the enclave

The agent never holds credentials. It makes calls to the gateway as if it were the upstream API. The gateway enforces policy, injects credentials, and forwards. The raw secret never crosses the enclave boundary.

---

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                        Agent VM                             │
│                                                             │
│   Agent process                                             │
│   curl http://169.254.x.x/gateway/github/repos/...  ──────►│
│                                                             │
└──────────────────────────┬──────────────────────────────────┘
                           │ link-local (169.254.x.x)
                           │ non-routable, host-only
                           ▼
┌─────────────────────────────────────────────────────────────┐
│              Gateway CVM (Confidential Enclave)             │
│                                                             │
│  ┌─────────────┐  ┌──────────────┐  ┌────────────────────┐ │
│  │ Attestation │  │ Policy Engine│  │  Credential Vault  │ │
│  │    Gate     │  │              │  │                    │ │
│  │             │  │ per-provider │  │ sealed to PCR/MRTD │ │
│  │ agent must  │  │ per-op rules │  │ never leaves CVM   │ │
│  │ prove binary│  │ allowlist    │  │                    │ │
│  └──────┬──────┘  └──────┬───────┘  └────────┬───────────┘ │
│         │                │                   │             │
│         └────────────────┼───────────────────┘             │
│                          │                                  │
│                    ┌─────▼──────┐                          │
│                    │  Forwarder │  ◄── injects credential  │
│                    │            │      into outbound req   │
│                    └─────┬──────┘                          │
│                          │                                  │
│              ┌───────────▼──────────────┐                  │
│              │    Audit Log (sealed)    │                  │
│              └──────────────────────────┘                  │
│                                                             │
└──────────────────────────┬──────────────────────────────────┘
                           │ TLS to upstream
                           ▼
              api.github.com / api.openai.com / etc.
```

---

## Components

### 1. Attestation Gate

Before any credential is used, the calling agent must establish an attested session:

- Agent generates a fresh session keypair and sends a CSR + TDX/SEV-SNP quote with the public key hashed into `reportdata`
- Gateway verifies the quote against expected MRTD/PCR measurements
- On success, issues a short-lived session token scoped to the allowed credential set
- Session tokens are non-transferable and bound to the agent instance

This provides **B2/K4-level enforcement** from the Practical CoCo framework: the gateway only activates for the exact binary it was configured to trust.

### 2. Credential Vault

- Credentials injected once at setup time via an attested operator channel
- Stored encrypted, sealed to the enclave's hardware measurement (MRTD for TDX, MEASUREMENT for SEV-SNP)
- Unsealed only inside the running enclave; never written in plaintext anywhere
- Optionally backed by a remote KBS (Key Broker Service) for rotation support

### 3. Policy Engine

The policy engine is the core novel contribution. Each credential has an associated policy document:

```yaml
credential: github-vinclaw
provider: github
base_url: https://api.github.com

rules:
  - allow:
      methods: [GET]
      path_pattern: "/repos/**"
  - allow:
      methods: [POST]
      path_pattern: "/repos/*/issues"
  - allow:
      methods: [GET, POST]
      path_pattern: "/user"
  - deny:
      methods: [DELETE]
      path_pattern: "/**"
  - deny:
      path_pattern: "/repos/*/hooks/**"   # no webhook manipulation
```

Rules are evaluated in order, first match wins. Denied requests are logged and rejected with a 403 — the upstream API is never contacted.

**Semantic extensions (future):**
- Content inspection: reject requests whose body matches patterns (e.g., exfil via gist content)
- Rate limiting per agent instance
- Time-of-day restrictions
- Cross-provider correlation (agent may not POST to GitHub and send email within the same 60s window)

### 4. Forwarder

- Rewrites the inbound agent request: strips gateway path prefix, injects `Authorization` header with the real credential
- Forwards over TLS to the upstream provider
- Streams response back to agent
- Records request metadata (timestamp, method, path, response code, bytes) to audit log

### 5. Audit Log

- Append-only log sealed inside the enclave
- Exportable to operator via attested channel (signed by enclave key, verifiable externally)
- Never contains credential values — only call metadata
- Enables post-incident forensics on agent behaviour

---

## Gateway API (Agent-facing)

The agent hits the gateway exactly as it would hit the upstream API, with a path prefix:

```
http://169.254.x.x/gateway/{provider}/{upstream_path}
```

Examples:

```bash
# GitHub — list repos
curl http://169.254.x.x/gateway/github/user/repos

# OpenAI — chat completion
curl http://169.254.x.x/gateway/openai/v1/chat/completions \
  -H "content-type: application/json" \
  -d '{"model": "gpt-4o", "messages": [...]}'

# ElevenLabs — TTS
curl http://169.254.x.x/gateway/elevenlabs/v1/text-to-speech/{voice_id} \
  -H "content-type: application/json" \
  -d '{"text": "...", "model_id": "eleven_multilingual_v2"}'
```

No API keys in the request. The gateway injects them. The agent's code is credential-free by design.

---

## Setup Flow

```
Operator                Gateway CVM               KBS (optional)
   │                        │                          │
   │── attest CVM ─────────►│                          │
   │◄─ quote + pubkey ──────│                          │
   │── verify quote ────────────────────────────────── │
   │                        │                          │
   │── inject credentials ─►│ (over attested channel)  │
   │                        │── seal to MRTD ──────────│
   │                        │── store encrypted ───────│
   │                        │                          │
   │── deploy policy docs ─►│                          │
   │                        │── load + compile rules ──│
   │                        │                          │
   │── register agent MRTD ►│                          │
   │                        │── store allowed measurements
   │                        │                          │
   [Gateway ready]
```

---

## CoCo Vector

Target deployment on bare-metal TDX or SEV-SNP (A3):

```
A3 | R4 | B2 | K4
```

- **A3** — direct silicon root of trust, no CSP paravisor
- **R4** — gateway built reproducibly (Nix + Rust), anyone can verify binary matches source
- **B2** — agent session keypair hashed into `reportdata`, verifier enforces match
- **K4** — credential release gated on exact agent MRTD + live session binding

Cloud deployment (e.g., GCP TDX) is also viable at `A3[GCP TDX] | R4 | B2 | K4` with explicit CSP trust declared.

---

## Implementation Plan

### Phase 1 — Proof of Concept
- [ ] Rust HTTP proxy, no policy engine, no attestation
- [ ] Single credential (GitHub token), hardcoded in config
- [ ] Link-local listener on `169.254.x.x`
- [ ] Basic path-prefix routing and credential injection
- [ ] Verify end-to-end: agent calls gateway, GitHub API responds

### Phase 2 — Policy Engine
- [ ] YAML policy document format + parser
- [ ] Rule evaluator (method + path pattern matching)
- [ ] Deny logging
- [ ] Unit test suite for policy evaluation

### Phase 3 — Credential Vault
- [ ] Encrypted credential store
- [ ] Seal/unseal to TEE measurement (MRTD/PCR)
- [ ] Attested operator injection channel
- [ ] Credential rotation support

### Phase 4 — Attestation Gate
- [ ] TDX/SEV-SNP quote verification (using `go-tdx-guest` or `sev-tool`)
- [ ] Agent MRTD allowlist
- [ ] Session token issuance + validation
- [ ] B2 binding: session pubkey in `reportdata`

### Phase 5 — Audit + Hardening
- [ ] Sealed append-only audit log
- [ ] Operator export via attested channel
- [ ] Network egress lockdown (gateway only routes to allowlisted upstream domains)
- [ ] Reproducible Nix build

---

## Tech Stack

- **Language:** Rust (memory safety, no GC pauses, excellent async HTTP with `hyper`/`axum`)
- **Build:** Nix (reproducible, R4-capable)
- **TEE:** Intel TDX (primary), AMD SEV-SNP (secondary)
- **Attestation:** `tdx-attest` crate / `sev` crate
- **Policy:** YAML config, compiled to rule tree at startup
- **Crypto:** `rustls` for TLS, `ring` or `aws-lc-rs` for sealing primitives

---

## Non-Goals (v1)

- mTLS between agent and gateway (link-local is sufficient boundary for v1)
- Multi-tenant (one gateway instance per agent deployment)
- GUI / management console
- Support for non-HTTP protocols (gRPC, SSH)

---

## Code References & Reuse

### [`vkobel/enclave-tls-api-fetcher`](https://github.com/vkobel/enclave-tls-api-fetcher) ⭐ Primary reference

A Nitro Enclave TLS fetcher with attestation, written in Rust. Large portions are directly reusable or adaptable:

| Crate / Module | Reuse in Gateway | Notes |
|---|---|---|
| `crates/enclave/src/tls/` | Outbound TLS to upstream APIs | TLS termination + proxy_io pattern inside enclave — use as-is |
| `crates/enclave/src/vsock/` | Enclave↔host communication channel | Swap for link-local if not using vsock; reuse otherwise |
| `crates/common/src/hpke.rs` | Attested operator injection channel | HPKE for credential bootstrap — reuse directly |
| `crates/common/src/protocol.rs` | Request/response framing | Adapt to gateway API surface |
| `crates/attestation-verifier/` | Attestation Gate (Phase 4) | PCR/quote verification library — reuse directly, swap NSM for TDX/SEV-SNP crates |
| `crates/host-proxy/` | Agent-facing HTTP API | Adapt: flip from fetcher API to proxy API |

**Key delta from this codebase:**
- Replace AWS Nitro NSM attestation with Intel TDX (`tdx-attest` crate) or AMD SEV-SNP (`sev` crate) — structure is identical
- Add inbound request routing (path-prefix → upstream mapping) instead of explicit URL parameter
- Add Policy Engine (new — YAML rules, method + path matching)
- Add Credential Vault (new — sealed encrypted store)

**Estimated savings:** Phases 1 and 4 of the implementation plan are largely already written here (~2–3 weeks of work reused).

---

## Related Work

- **exe.dev LLM Gateway** — inspiration for the link-local proxy pattern; scoped to LLM providers, no attestation, no policy
- **Phala Network tappd** — TEE-based secret management for Web3 workloads
- **Practical CoCo Framework** — scoring framework for evaluating this deployment's verifiability posture

---

*Spec draft — April 2026*
