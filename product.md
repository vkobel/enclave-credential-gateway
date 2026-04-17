# CoCo Credential Gateway — Product Vision

## What this project is today

A Rust HTTP proxy that stands between AI agents and upstream APIs (OpenAI,
Anthropic, GitHub, …). Agents authenticate with a **phantom token** — a shared
secret that is worthless outside the gateway. The gateway validates it in
constant time, strips it, injects the real upstream credential, and streams
the response back. Real keys never touch the agent's host.

The gateway is built on the `nono-proxy` library (phantom token pattern,
route store, credential injection) and extends it into a **remotely deployable,
hardware-attested** service. Routes are defined in a JSON profile
(`examples/profile.json`), so any Bearer-style upstream can be added without
code changes. The current POC (Phase 1a) runs as a Docker Compose workload on
any Linux host; Phase 1b (in progress) promotes the same binary to a Phala
Cloud TDX Confidential VM and exposes a `GET /attest` endpoint that returns a
raw TDX DCAP QuoteV4 so operators can verify the binary running in the
enclave before they entrust it with credentials.

The core insight: **credentials are infrastructure, not agent state.** An
agent should prove it is allowed to call an upstream, not hold the key to
that upstream. This gateway is the smallest possible control plane that makes
that true — and, once sealed in a TEE, makes it verifiably true.

---

## Vision — 2 weeks out

**"The POC is real, attested, and something a second person can use."**

- Phase 1b (`poc-v1b-cvm-attestation`) is complete: the same binary runs on a
  Phala Cloud TDX CVM, `GET /attest` returns a verified non-debug QuoteV4,
  and `OPENAI_API_KEY` / `ANTHROPIC_API_KEY` are provisioned via Phala's
  encrypted secret channel — never visible to the host operator.
- A published GHCR image (via `.github/workflows/docker.yml`) so anyone can
  `docker compose up` against it without building from source.
- `DEPLOY.md` walks an operator from "I have a Phala account" to "Claude Code
  is talking to the gateway and my key is in the TEE" in under 15 minutes.
- Live end-to-end demo: Claude Code on a fresh laptop, `ANTHROPIC_API_KEY`
  set only to the phantom token, full chat session succeeds, `grep` on the
  laptop's process tree and outbound traffic confirms the real
  `sk-ant-…` never leaves the CVM.
- Egress enforcement gap documented honestly — with concrete mitigation
  recipes (cloud egress firewall, eBPF filter) and a clearly labeled pointer
  to "Path C" as the real fix.

**Success metric:** one external developer deploys their own instance from
the docs alone and sends a chat completion through it, and the attestation
quote they pulled verifies offline against Intel's PCS.

---

## Vision — 2 months out

**"A credible credential control plane for agent fleets, not a single
agent."**

The POC becomes a product that a small team can actually run. Phase 2 work
lands:

- **Policy engine.** Per-route, per-token rules: rate limits, token budgets
  (e.g. `anthropic: 50k tokens/day`), upstream path allowlists
  (`/openai/v1/chat/completions` yes, `/openai/v1/files` no), request size
  caps. Policy config lives beside the route profile, hot-reloadable.
- **Per-agent identity, not one shared phantom.** Short-lived, scoped
  phantom tokens minted by the gateway (or a companion issuer) — each agent
  gets its own token, bound to a policy bundle, with an expiry. Token
  revocation is O(1) without redeploying.
- **Audit log.** Every proxied request: timestamp, phantom-token ID (not
  value), route, upstream path, response status, bytes in/out, approximate
  token count for LLM routes. Append-only, streamable to an external sink
  (stdout JSON → Loki/Datadog, or a structured S3 bucket).
- **Encrypted vault abstraction.** Credentials are no longer just env vars;
  they live in a portable encrypted blob, unsealed inside the TEE at boot.
  Phala secrets remain the default unseal path, but the vault format is
  portable so the same blob can eventually target Azure TDX, GCP
  Confidential, AWS Nitro.
- **Multi-platform attestation.** `GET /attest` grows a platform-agnostic
  shape — still raw quote underneath, but with a thin
  `attestation-rs`-backed layer so the same codepath works on more than
  Phala.
- **First-class streaming and tool-use correctness.** SSE, chunked
  transfers, Anthropic's event stream, and OpenAI's tool-use round trips all
  have explicit integration tests in CI, including under provider-side
  back-pressure and mid-stream upstream errors.
- **Observability.** `/metrics` (Prometheus): request rate by route, 4xx/5xx
  breakdown, upstream latency p50/p95/p99, budget consumption per token ID.

**Success metric:** a team of 5 engineers runs 20+ agents against a single
gateway instance for a week, nobody on the team holds a raw provider key on
their laptop, and the ops person can answer "who spent my Anthropic budget
yesterday?" from the audit log in under a minute.

---

## Vision — 2 years out

**"The default way agents hold credentials — the way containers held
secrets after Vault."**

CoCo Credential Gateway is the boring, obvious answer to "how does an AI
agent authenticate?" the same way Vault became the boring answer for
microservices. The product spans three layers:

1. **The gateway itself** — still small, still Rust, still built on the
   phantom-token pattern, but hardened to run as shared infrastructure:
   multi-tenant, HA, horizontally scalable, with proper key rotation,
   N-of-M unseal, and reproducible builds whose `MRTD` is pinned in public
   release notes. In-enclave TLS termination (client-auth and mTLS with
   attested certificates) is the default, not a deferred item.
2. **The policy and identity plane** — rich policy language (who, what,
   when, how much, from where), OIDC-style issuance of phantom tokens from
   existing corporate identity, per-agent cost and capability budgets,
   and bring-your-own policy engine via a stable evaluation interface (Rego,
   Cedar, or a native DSL — the gateway doesn't care).
3. **The ecosystem around it** — a registry of audited route profiles for
   common upstreams (every major LLM provider, every major dev API), an SDK
   shim that makes *every* popular agent framework "CoCo-aware" with one
   config line, a remote-attestation verifier library and CLI so relying
   parties can gate access on a fresh quote, and a hosted "CoCo Cloud"
   offering for teams who want the gateway without running the TEE
   themselves.

The hard property the 2-year product delivers: **you can give an agent the
ability to act without giving it the ability to steal.** The agent never has
the raw credential. The credential never leaves attested hardware. Every
call is policy-checked and audit-logged. Revocation is instant. And the
person running the agent can prove, to a third party, exactly which binary
holds their keys.

Adjacent outcomes that become possible once the core is in place:

- **Delegation and capability handoff** — an agent can pass a narrower
  phantom token to a sub-agent or tool call, and the narrowing is enforced
  by the gateway, not by the sub-agent.
- **Per-call cryptographic receipts** — the gateway signs a statement "phantom
  token X called route Y at time T, upstream returned status Z" with an
  attested key. Useful for compliance, billing, and dispute resolution.
- **Credential-less CI and local dev** — developers run agents against a
  shared team gateway; nobody copies a provider key to a laptop, ever.
  Revoking an ex-employee's access is deleting one row.
- **Cross-cloud portability** — the same encrypted vault blob unseals on
  Phala, Azure, GCP, AWS; operators choose their TEE substrate without
  re-provisioning secrets.

**Success metric (2Y):** when a security team at a regulated company asks
"how are our agents handling API keys?", the answer — in industry
documentation, in vendor security questionnaires, and in the default setup
of at least two major agent frameworks — is "through a credential gateway,
probably CoCo." At that point the project has moved from "clever POC" to
"assumed infrastructure."

---

## Unified Deployment Model: Three Modes, One Protocol

The phantom token pattern is transport-agnostic. The same `Proxy-Authorization: Bearer <phantom>` header works whether the proxy is on the same host, a hardened local process, or a remote attested CVM. We support three deployment modes, unified by identical credential injection semantics:

| Mode | Topology | Trust Anchor | Use Case |
|------|----------|--------------|----------|
| **Remote** (Phase 1) | Agent → Network → CoCo Gateway (Phala TDX) | TDX DCAP QuoteV4 | Corporate shared infra, untrusted agent host |
| **Local** (Phase 2) | Agent → localhost:PORT (Nono sandbox) | Process isolation + macOS seatbelt | Developer laptop, single-user |
| **Enclave** (Phase 3) | Agent → localhost:PORT (hardened binary) | Secure Enclave + Hardened Runtime + Apple attestation | High-sensitivity local workloads |

All three speak the same JSON profile format, validate phantom tokens identically, and can be targeted by the same agent SDK with only a config change.

### Remote Mode (Current — Phase 1)

What we're building now. The gateway runs as a Phala Cloud TDX CVM, exposed over HTTPS with attestation available at `GET /attest`. The agent host is completely untrusted — compromise of the laptop reveals only the phantom token, not the upstream credentials. Attestation is mandatory; the operator verifies the quote before provisioning secrets into the CVM.

**Configuration:**
```yaml
# ~/.coco/config
mode: remote
gateway_url: https://coco-gateway.example.com
phantom_token: coco-phantom-abc123
verify_attestation: true
```

### Local Mode (Next — Phase 2)

Integration with [Nono](https://nono.sh) — the agent runs inside Nono's capability-based sandbox, with the CoCo Gateway binary as a sidecar process on the same host. Network egress from the sandbox is forced through the local proxy; the sandbox policy denies direct outbound connections.

This gives process-level isolation without a network hop. The phantom token is still required, but now the proxy is local and the attestation is implicit (same-host IPC). The agent can see the proxy's process but cannot extract credentials from it due to the sandbox boundary.

**Why Nono:** Nono already implements the phantom token pattern, route store, and credential injection. Rather than reimplement, we consume Nono's proxy library (or contribute upstream) and add our TDX-aware gateway as an additional backend. Nono users get TEE-grade remote attestation as an opt-in upgrade path; CoCo users get local sandboxing as a dev-mode convenience.

**Configuration:**
```yaml
mode: local
nono_profile: ~/.nono/profiles/secure.yml
phantom_token: coco-phantom-abc123
# No gateway_url — proxy is localhost via Nono
```

### Enclave Mode (Phase 3)

The hardened macOS approach from [d-inference](https://github.com/vkobel/d-inference): a Rust binary signed with Hardened Runtime, using the Secure Enclave for key operations, with attestation via Apple Device Attestation (MDM) or custom challenge-response. This is the "Private Cloud Compute for the rest of us" — hardware-backed assurance that the proxy binary hasn't been tampered with, without needing a remote CVM.

**Why this matters:** Some workloads can't leave the machine (airgapped, legal hold, classified). Others need the lowest possible latency. Enclave mode gives you attestation-backed credential isolation without network dependency.

**Configuration:**
```yaml
mode: enclave
binary_path: /opt/coco/coco-gateway-enclave
attestation: apple-secure-enclave
phantom_token: coco-phantom-abc123
```

### Migration Path

All three modes share the **phantom token protocol**, enabling seamless migration:

1. **Start with Remote** (Phase 1): Deploy to Phala, validate the model with real users, establish trust in the attestation flow.
2. **Add Local** (Phase 2): Developer installs Nono, runs `coco mode local`, same phantom token now routes through local sandbox instead of remote CVM. Faster iteration, same security model.
3. **Upgrade to Enclave** (Phase 3): For the paranoid, swap the local Nono proxy for the hardened enclave binary. Same config, stronger attestation.

The agent SDK handles this transparently:
```bash
coco run --mode auto  # Detects: Remote if configured, Local if Nono present, Enclave if binary available
```

## Non-goals, at every horizon

- **Not a general HTTPS forward proxy.** We only proxy explicitly configured
  upstreams with well-understood credential shapes. Generic `CONNECT` is out.
- **Not a model router.** We don't pick models, rewrite prompts, cache
  completions, or do semantic routing. That's a different product; keeping
  this one dumb is a feature.
- **Not a replacement for Vault or a cloud KMS** for non-agent workloads.
  The gateway is *specifically* the answer for "agent → external API"
  traffic. Other secret management needs keep using the tools that already
  solve them.
- **Not a silver bullet for prompt-injection or agent misbehavior.**
  Holding the credential outside the agent limits blast radius; it does not
  make the agent trustworthy. Policy and budgets are how we keep a
  misbehaving agent from burning the house down, not how we stop it from
  misbehaving.
- **Not a fork of Nono.** We integrate with Nono, we don't replace it. The
  Nono project owns sandboxing; we own remote attestation and TEE deployment.
  Where our work can upstream (hardened runtime mode, Secure Enclave key
  generation), we contribute back.
