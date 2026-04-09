# CoCo Credential Gateway

> A TEE-hardened universal API & LLM credential proxy for AI agents, featuring portable vaults, semantic policy enforcement, and remote attestation.

---

## 🛑 The Problem

AI agents require credentials to be useful — GitHub tokens, OpenAI API keys, payment APIs, and communication services. Today, these credentials are fundamentally vulnerable:

1. **Host Environment Compromise:** Keys baked into laptops, cloud VMs, or CI runners are visible to compromised dependencies, memory dumps, or supply chain attacks.
2. **Configuration Fatigue:** Users of tools like OpenClaw or Claude Desktop constantly wrestle with reconfiguring API keys across different platforms and providers.
3. **Inadequate Partial Solutions:**
   - **Secret Managers (Vault):** Protect secrets at rest but hand them to the agent in plaintext at runtime.
   - **LLM Gateways (LiteLLM, OpenRouter):** Abstract LLM keys but lack hardware-level guarantees, don't cover non-LLM APIs (e.g., GitHub), and require trusting a third-party host.
   - **Local Sandboxes (Nono):** Great for local "phantom token" injection, but tied to a single machine's OS keychain with no remote attestation or cross-machine portability.

**An agent that can make arbitrary API calls with injected credentials can still do harm, even if it never sees the raw key.** Without a robust, network-accessible, hardware-attested proxy, the agent remains the weakest link.

---

## 💡 The Solution

**CoCo Credential Gateway** is a remotely accessible credential proxy running inside a Confidential VM (CVM) or TEE (Intel TDX, AMD SEV-SNP, AWS Nitro). It acts as an **AI Firewall and Portable Vault**, intercepting agent requests, evaluating them against semantic policies, injecting the real credentials, and forwarding them to the upstream provider.

### Core Value Pillars

1. **The "Phantom Token" Pattern, Elevated:** Agents authenticate to CoCo using identity-based session tokens. CoCo swaps these for the real API keys inside the enclave. The raw secrets _never_ touch the agent's host machine.
2. **Universal LLM & API Routing:** Acts as a drop-in `BASE_URL` replacement. It provides an OpenAI-compatible endpoint for LLMs (integrating patterns from LiteLLM) _and_ standard proxying for REST APIs (GitHub, Stripe, etc.).
3. **Portable Encrypted Vaults:** Users hold a master passphrase or hardware key. They can export their encrypted vault blob and drop it into any new CoCo instance. The vault only decrypts if the gateway's hardware attestation is verified.
4. **Semantic AI Firewall:** Beyond basic method/path blocking, CoCo evaluates the _intent_ of a request (e.g., "Block any request attempting to delete a production repository" or "Block transactions over $5,000 unless initiated by identity `ops-lead`" in Fintech use cases).
5. **Verifiable Hardware Trust:** Built reproducibly (Nix + Rust), terminating TLS _inside_ the enclave, with cryptographic proof of the gateway's integrity.

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

### 2. Universal LLM Gateway Layer

Instead of managing API keys for Claude, OpenAI, and Ollama, the agent points its `OPENAI_BASE_URL` to the CoCo Gateway. CoCo routes the request, injects the correct provider key from the unlocked vault, and enforces cross-provider token budgets.

### 3. Portable Encrypted Vaults

The biggest UX friction in agent deployments is key configuration. CoCo allows users to maintain a single encrypted vault file (e.g., `coco-vault.enc`). When spinning up a new CoCo instance on AWS Nitro or GCP TDX, the user provides the vault file and a decryption key. The enclave decrypts the vault into volatile memory. If the enclave reboots, the secrets are gone until unlocked again.

### 4. Semantic AI Firewall

Standard proxies block `/delete`. CoCo goes further. Using a lightweight local model or a trusted upstream call, the firewall evaluates the _payload intent_:

- **Fintech/DeFi:** "Deny uncollateralized loan API execution."
- **DevOps:** "Block exfiltration of files matching `AWS_`."
- **General:** "Limit context window to 10k tokens for identity `ci-bot`."

### 5. Identity-Gated Agents

Agents use simple "phantom tokens" (like Nono) or mTLS certs. If an agent goes rogue, the operator simply revokes the phantom token. The real upstream credentials (which might be shared across multiple agents) never need rotating.

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

- **Phase 1 (PoC):** Rust HTTPS server in TDX/Nitro, terminating TLS in-enclave. Basic path-prefix routing and static token injection.
- **Phase 2 (LLM Gateway):** Implement OpenAI-compatible endpoints (`/v1/chat/completions`) and token budget enforcement. Credentials are loaded from config files at this stage (vault comes in Phase 3).
- **Phase 3 (Portable Vault):** Build the encrypted vault import/export logic and operator unlock channel. Replaces the config-file credentials from Phase 2.
- **Phase 4 (Semantic Firewall):** Integrate intent-based blocking via lightweight embedded policy evaluation.
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

### Extending for CLI Tool Proxying (e.g., `gh`, `stripe`)

CLI tools don't all use HTTP proxies. Three patterns to cover:

1. **Env var injection (works today):** Tools like `gh` read `GH_TOKEN` from env. Nono's keystore loads secrets and injects them as env vars into the sandboxed process. The sandbox prevents exfiltration.

2. **HTTP proxy mode (works today):** Tools respecting `HTTPS_PROXY` are routed through `nono-proxy` CONNECT tunneling or reverse proxy. Configure `HTTPS_PROXY=http://127.0.0.1:<port>` in the sandbox env.

3. **Config file generation (gap):** Tools that read credentials from config files (e.g., `~/.config/gh/hosts.yml`) need a pre-exec step that writes a temp config with injected credentials inside the sandbox. This is a new capability to build — a config template system that resolves credential refs at launch time.

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
2. **Add TLS termination + remote client auth.** Replace localhost-only binding with attested TLS. Session tokens become identity-bearing (JWT or macaroons).
3. **Build the vault layer on top of existing keystore abstractions.** `CredentialStore` and `LoadedCredential` already handle multi-backend secret loading — wrap with encrypt/export/import.
4. **Wrap in TEE runtime.** Deploy into Nitro/TDX CVM. Wire attestation report to TLS public key. Gate vault decryption on measurement verification.
5. **Add semantic firewall as a new policy layer.** Sits alongside the existing `CompiledEndpointRules` in `RouteStore` — same hook point, richer evaluation.

---

_April 2026_
