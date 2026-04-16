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
