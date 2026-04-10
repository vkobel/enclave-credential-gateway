# CoCo Credential Gateway

> A confidential, verifiable credential proxy for AI agents: phantom-token injection, policy enforcement, and attested execution inside a TEE.

---

## 🛑 The Problem

AI agents require credentials to be useful — GitHub tokens, OpenAI API keys, payment APIs, and communication services. Today, these credentials are fundamentally vulnerable:

1. **Host Environment Compromise:** Keys baked into laptops, cloud VMs, or CI runners are visible to compromised dependencies, memory dumps, or supply chain attacks.
2. **Configuration Fatigue:** Users of tools like OpenClaw or Claude Desktop constantly wrestle with reconfiguring API keys across different platforms and providers.
3. **Inadequate Partial Solutions:**
   - **Secret Managers (Vault):** Protect secrets at rest but hand them to the agent in plaintext at runtime.
   - **LLM Gateways (LiteLLM, OpenRouter):** Abstract LLM keys but lack hardware-level guarantees, don't cover non-LLM APIs (e.g., GitHub), and require trusting a third-party host.
   - **Local Sandboxes (Nono):** Great for local "phantom token" injection, but tied to a single machine's OS keychain with no remote attestation or cross-machine portability.

**An agent that can make arbitrary API calls with injected credentials can still do harm, even if it never sees the raw key.** The right primitive is not "give the agent the secret more carefully." The right primitive is a **credential firewall**: a proxy that holds the real credentials, enforces policy at the request boundary, and is remotely verifiable.

---

## 💡 The Solution

**CoCo Credential Gateway** is a remotely accessible credential proxy running inside a Confidential VM (CVM) or TEE (Intel TDX, AMD SEV-SNP, AWS Nitro). It takes the local phantom-token pattern already proven by `nono` and promotes it into a **confidential, verifiable network service**: agents authenticate to CoCo with session or workload identity, CoCo injects the real upstream credential inside the enclave, and forwards the request without ever exposing the raw secret on the agent's host.

The core thesis is intentionally narrower than "handle every auth shape for every tool." CoCo is first and foremost a **confidential reverse proxy for HTTP-facing agent credentials**: LLM providers, SaaS APIs, GitHub/GitLab/Stripe-style REST APIs, and OpenAI-compatible endpoints. Compatibility shims for non-HTTP-native tools are useful later, but they are not the defining primitive.

### Core Value Pillars

1. **The "Phantom Token" Pattern, Elevated:** Agents authenticate to CoCo using identity-based session tokens or mTLS. CoCo swaps these for the real upstream credentials inside the enclave. The raw secrets _never_ touch the agent's host machine.
2. **Confidential + Verifiable Execution:** TLS terminates inside the enclave. The gateway's public key is bound into the attestation evidence, so the operator can verify both the binary identity and the endpoint they are talking to.
3. **HTTP-Native Credential Firewall:** CoCo acts as a drop-in `BASE_URL` replacement for LLMs and a standard reverse proxy for HTTP APIs. This is the main primitive, not an incidental transport.
4. **Policy at the Request Boundary:** Method/path allowlists, identity scoping, endpoint-specific rules, token budgets, and later structured or semantic checks happen where the secret is actually used.
5. **Portable Vaults as an Enabler, Not the Product:** Portable encrypted vaults make the service usable across machines and clouds, but the core product remains the confidential proxy boundary.

---

## 🏗️ Architecture

```text
┌─────────────────────────────────────────────────────────────────────────┐
│                          Agents (Any Platform)                          │
│                                                                         │
│    Laptop (OpenClaw)       Cloud VM (CrewAI)         CI Runner          │
│            │                       │                     │              │
│            └───────────────────────┴─────────────────────┘              │
│                                    │                                    │
│                     HTTPS + Phantom Token / mTLS                        │
│                                    ▼                                    │
└─────────────────────────────────────────────────────────────────────────┘
                                     │
                                     │ Attested TLS (terminates in CVM)
                                     ▼
┌─────────────────────────────────────────────────────────────────────────┐
│                 Gateway CVM (Confidential Enclave)                      │
│                                                                         │
│  ┌────────────────────┐   ┌──────────────────────────────────────────┐  │
│  │ Attested TLS       │   │           Portable Vault Core            │  │
│  │ Endpoint           │   │                                          │  │
│  │ (Pubkey in quote)  │   │  User Unlocks ──► Decrypts into memory   │  │
│  └─────────┬──────────┘   │  Sealed to hardware measurement (MRTD)   │  │
│            │              └────────────────────┬─────────────────────┘  │
│            ▼                                   │                        │
│  ┌────────────────────┐   ┌────────────────────▼─────────────────────┐  │
│  │ Agent Identity Gate│──►│            Policy Engine                 │  │
│  │ (Maps phantom token│   │ - YAML path/method rules                 │  │
│  │  to policy scope)  │   │ - Semantic LLM Firewall (Intent blocks)  │  │
│  └────────────────────┘   │ - Token budget enforcement               │  │
│                           └────────────────────┬─────────────────────┘  │
│                                                │                        │
│  ┌─────────────────────────────────────────────▼─────────────────────┐  │
│  │                     Universal Forwarder                           │  │
│  │                                                                   │  │
│  │   [ LLM Gateway Layer ]           [ Standard API Proxy ]          │  │
│  │  OpenAI-compatible routing         GitHub, Stripe, Zendesk        │  │
│  │  (LiteLLM architecture)                                           │  │
│  └─────────────────────────────────────────────┬─────────────────────┘  │
│                                                │                        │
│  ┌─────────────────────────────────────────────▼─────────────────────┐  │
│  │                 Sealed Audit Log (Tamper-evident)                 │  │
│  └───────────────────────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────────────────────┘
                                     │
                                     │ TLS to Upstream
                                     ▼
             api.openai.com / api.github.com / api.stripe.com
```

---

## ⚙️ Key Components

### 1. Attested TLS & Reproducible Builds

CoCo generates its TLS keypair inside the enclave at startup. The public key is hashed into the hardware attestation report (`reportdata`). Before unlocking the portable vault, the user verifies the attestation quote, proving the binary matches the reproducible open-source build and the TLS connection is secure from MITM attacks.
_(Vector: A3 | R4 | B2 | K4)_

### 2. Confidential Reverse Proxy Layer

Instead of managing API keys for Claude, OpenAI, GitHub, Stripe, or internal REST services directly on the agent host, the agent points its `BASE_URL` or provider config to the CoCo Gateway. CoCo routes the request, injects the correct credential from the unlocked vault, and enforces policy before the request leaves the enclave.

This is the most important scope clarification: CoCo is primarily an **attested reverse proxy**, not a generic secret-injection framework for every local CLI auth convention.

### 3. Portable Encrypted Vaults

The biggest UX friction in agent deployments is key configuration. CoCo allows users to maintain a single encrypted vault file (e.g., `coco-vault.enc`). When spinning up a new CoCo instance on AWS Nitro or GCP TDX, the user provides the vault file and a decryption key. The enclave decrypts the vault into volatile memory. If the enclave reboots, the secrets are gone until unlocked again.

### 4. Policy Engine

Standard proxies block `/delete`. CoCo should start one layer earlier and simpler: enforce deterministic policy on method, path, identity, provider, and budget. A later phase can add deeper semantic checks using a lightweight local model or a trusted upstream call:

- **Fintech/DeFi:** "Deny uncollateralized loan API execution."
- **DevOps:** "Block exfiltration of files matching `AWS_`."
- **General:** "Limit context window to 10k tokens for identity `ci-bot`."

### 5. Identity-Gated Agents

Agents use simple "phantom tokens" (like Nono) or mTLS certs. If an agent goes rogue, the operator simply revokes the phantom token. The real upstream credentials (which might be shared across multiple agents) never need rotating.

## 🎯 Scope Clarification

The first and strongest version of CoCo is:

- a **remote reverse proxy**
- that holds the real credentials inside a TEE
- that accepts only identity-bearing phantom tokens or mTLS-authenticated clients
- that exposes provider-shaped HTTP endpoints to agents
- and that can be cryptographically verified before it is trusted

That means Pattern 1 from the POC note is not just "one option." It is the core product.

For many agent workloads, that is already enough:

- OpenAI and OpenAI-compatible APIs
- Anthropic
- GitHub REST / GraphQL
- Stripe
- Slack
- Linear
- Notion
- internal HTTP services

Compatibility layers for tools that only read env vars or config files may still be useful, but they are **secondary adapters**, not the main thesis. If a tool can be made to talk to a provider-shaped HTTP endpoint, CoCo should prefer that path.

---

## 🚀 Use Cases & Business Opportunities

- **Enterprise AI Rollouts:** Prevent "Shadow AI" credential leaks. Centralize auditing and token budgeting across hundreds of employee agents.
- **Fintech & Web3:** Securely execute programmatic trading or unsecure loan issuance where the agent logic cannot be trusted with direct API access.
- **Consumer Agent Portability:** Let users bring their own LLM subscriptions to any cloud agent platform (CrewAI, OpenClaw) without exposing their raw Anthropic/OpenAI keys to the platform provider.

---

## 🔗 Code References & Reuse

### [`vkobel/enclave-tls-api-fetcher`](https://github.com/vkobel/enclave-tls-api-fetcher) — Primary reference

A Nitro Enclave TLS fetcher with attestation, written in Rust. Large portions are directly reusable or adaptable:

| Crate / Module | Reuse in Gateway | Notes |
|---|---|---|
| `crates/enclave/src/tls/` | TLS termination inside enclave | Adapt for inbound TLS (server), reuse outbound TLS to upstream |
| `crates/enclave/src/vsock/` | Enclave↔host communication channel | Swap for TCP if not using vsock |
| `crates/common/src/hpke.rs` | Attested operator injection channel | HPKE for credential bootstrap — reuse directly |
| `crates/common/src/protocol.rs` | Request/response framing | Adapt to gateway API surface |
| `crates/attestation-verifier/` | Attestation endpoint (Phase 5) | PCR/quote verification library — reuse, swap NSM for TDX/SEV-SNP |
| `crates/host-proxy/` | Agent-facing HTTPS API | Adapt: add API key/mTLS auth layer |

**Key delta from this codebase:**
- Add inbound TLS termination (HTTPS server inside enclave)
- Replace AWS Nitro NSM attestation with Intel TDX / AMD SEV-SNP
- Add Policy Engine (YAML rules, method + path matching)
- Add Credential Vault (sealed encrypted store)

**Estimated savings:** Phases 1 and 5 are significantly de-risked by this reference (~2–3 weeks of work reused).

---

## 📚 Related Work & Inspirations

CoCo stands on the shoulders of several excellent OSS projects, combining their concepts into a hardware-secured paradigm:

1. **[Nono](https://github.com/lukehinds/nono):** The pioneer of the "phantom token" and local agent sandboxing pattern. CoCo takes Nono's credential injection and makes it remote, multi-tenant, and hardware-attested.
2. **[LiteLLM](https://github.com/BerriAI/litellm):** The standard for OpenAI-compatible LLM routing. CoCo integrates this routing logic inside the TEE.
3. **[Deepsecure](https://github.com/DeepTrail/deepsecure):** Reference for macaroons-based delegation and semantic policy scopes.
4. **[Phala dstack & Kettle](https://github.com/Phala-Network/dstack):** Inspiration for TEE attestation flows and cloud CVM deployment patterns (Azure/GCP TDX).

---

## 🛣️ Implementation Roadmap

- **Phase 1 (PoC):** Rust HTTPS reverse proxy in TDX/Nitro, terminating TLS in-enclave. Basic path-prefix routing, phantom-token validation, and static credential injection for HTTP APIs.
- **Phase 2 (Policy + Identity):** Add OpenAI-compatible endpoints, identity-bearing tokens or mTLS, deterministic method/path policy, and token budget enforcement.
- **Phase 3 (Portable Vault):** Build the encrypted vault import/export logic and operator unlock channel. Replace static bootstrap credentials with an attested vault.
- **Phase 4 (Compatibility Adapters):** Add optional wrappers for tools that cannot use provider-shaped HTTP endpoints directly.
- **Phase 5 (Audit & Attestation):** `/.well-known/attestation` endpoints, reproducible Nix builds, and tamper-evident logging.

---

## 🔍 Nono as a Foundation — Gap Analysis

CoCo doesn't need to start from scratch. The [nono](https://github.com/lukehinds/nono) project's `nono-proxy` crate already implements several core CoCo primitives as a local sidecar. Below is an honest assessment of what transfers directly, what needs extending, and what's genuinely new.

### Already Implemented in Nono

| CoCo Concept              | Nono Implementation                                                                                         | Notes                                                               |
| ------------------------- | ----------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------- |
| **Phantom Token Pattern** | `nono-proxy/reverse.rs` — validates session token, strips it, injects real credential from keystore         | Multiple injection modes: header, URL path, query param, basic auth |
| **Universal API Routing** | `nono-proxy/route.rs` — path-prefix routing (`/openai/...` → `api.openai.com`)                              | LLM and non-LLM APIs handled identically                            |
| **L7 Endpoint Filtering** | `RouteStore` with per-route method+path rules (default-deny when configured)                                | Foundation for the AI Firewall's rule layer                         |
| **Credential Backends**   | `keystore.rs` — macOS Keychain, Linux Secret Service, 1Password (`op://`), Apple Passwords, env vars, files | Secrets wrapped in `Zeroizing<String>`                              |
| **Host Filtering**        | `net_filter.rs` — allowlists, hardcoded cloud metadata deny (169.254.x.x), DNS rebinding protection         | Non-overridable deny list                                           |
| **Audit Logging**         | `SharedAuditLog` — session metadata capture per proxied request                                             | Needs tamper-evidence for CoCo                                      |
| **Sandbox Isolation**     | Landlock (Linux) + Seatbelt (macOS) — agent network locked to proxy-only, filesystem restricted             | Prevents credential exfiltration at OS level                        |

### What This Proves About CoCo's Core Primitive

`nono` already proves the local version of the design I actually care about:

1. the agent talks to a local provider-shaped HTTP endpoint
2. the agent holds only a phantom token, not the real secret
3. the proxy validates the phantom token and injects the real credential
4. the proxy can enforce per-endpoint policy before forwarding upstream

That means the main CoCo delta is not inventing a new credential pattern. It is making this pattern:

- remote instead of localhost-only
- attested instead of host-trust-based
- identity-aware instead of single-user
- portable instead of tied to one machine's keychain

### Secondary Compatibility Paths

For completeness: not every tool is naturally provider-shaped or HTTP-native. Some still rely on env vars, HTTP proxy settings, or config files. Those paths may still matter for adoption, but they are secondary to the core proxy thesis.

- **Env var injection:** Works today in `nono`, but it is weaker because the secret enters the process environment.
- **HTTP proxy mode:** Works today for proxy-aware tools, but it is less expressive than service-specific reverse proxying.
- **Config file generation:** Still a gap and still weaker than phantom-token reverse proxying because the real credential becomes locally readable.

### Gaps — What CoCo Must Build New

| CoCo Feature                  | Gap                                                                                                       | Effort Estimate                                                  |
| ----------------------------- | --------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------- |
| **Remote network listener**   | Nono binds localhost only. CoCo needs authenticated remote access with TLS.                               | Medium — promote bind config, add client auth                    |
| **TEE/CVM execution**         | Entirely new. In-enclave TLS termination, sealed memory, hardware attestation.                            | Large — platform-specific (Nitro, TDX, SEV-SNP)                  |
| **Portable encrypted vaults** | Nono reads local keystores. CoCo needs exportable encrypted blobs sealed to hardware measurements (MRTD). | Large — new crypto layer + key derivation                        |
| **Remote attestation**        | `/.well-known/attestation` endpoint, quote generation/verification, reproducible build hashes.            | Large — per-TEE-platform attestation flows                       |
| **Semantic AI Firewall**      | Nono has path/method rules. CoCo wants LLM-evaluated intent blocking.                                     | Medium — new policy evaluation layer alongside existing L7 rules |
| **Multi-tenancy**             | Nono is single-user. CoCo needs identity mapping, per-agent policy scopes, token budget tracking.         | Medium — extend existing session token to carry identity claims  |

### Recommended Build Path

1. **Extract `nono-proxy` into a standalone service.** It already runs as a separate unsandboxed process with its own async runtime. Minimal refactoring to make it independently deployable.
2. **Add TLS termination + remote client auth.** Replace localhost-only binding with attested TLS. Session tokens become identity-bearing (JWT, macaroons, or mTLS-backed identities).
3. **Keep the reverse-proxy data plane as the product core.** Do not widen the first version into every CLI auth shape. Optimize for HTTP-facing agent credentials first.
4. **Build the vault layer on top of existing keystore abstractions.** `CredentialStore` and `LoadedCredential` already handle multi-backend secret loading — wrap with encrypt/export/import.
5. **Wrap in TEE runtime.** Deploy into Nitro/TDX CVM. Wire attestation report to TLS public key. Gate vault decryption on measurement verification.
6. **Add richer policy only after the core proxy is proven.** Start with deterministic endpoint policy; add semantic evaluation later if it proves necessary.

---

_April 2026_
