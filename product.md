# CoCo Credential Gateway — Product Vision

## What this is

A Rust HTTP proxy that stands between AI agents and upstream APIs (OpenAI,
Anthropic, GitHub, …). Agents authenticate with a **phantom token** — a
scoped, revocable, short-lived credential that is worthless outside the
gateway. The gateway validates it in constant time, strips it, injects the
real upstream credential, and streams the response back. Real keys never
touch the agent's host.

The gateway is built on the `nono-proxy` library and runs inside a Phala
Cloud TDX Confidential VM. A `GET /attest` endpoint returns a raw TDX DCAP
QuoteV4 so any caller can verify the exact binary holding the credentials
before trusting it.

The core insight: **credentials are infrastructure, not agent state.**

---

## Who this is for

**Regulated teams running AI agents against paid APIs.** Financial services,
healthcare, legal, and EU-AI-Act-scoped companies share four problems no
current tool answers together:

1. **Custody.** Production API keys must not live on developer laptops or
   CI runners. Today they do.
2. **Attribution.** "Which person, on which device, through which policy,
   made this call?" — must be answerable from an audit log.
3. **Revocation.** An employee leaves; a laptop is lost; an agent
   misbehaves. Access must be cut in seconds, without redeploying.
4. **Proof.** Auditors and regulators need third-party-verifiable evidence
   of the above, not vendor assurances.

These teams already run MDM (Jamf, Kandji, Intune), SSO (Okta, Entra),
and corporate CAs. They will adopt new infrastructure that slots into what
they have — not a parallel identity system.

---

## The bet: mutual attestation

A plain remote proxy answers custody and attribution. It does not answer
**"is the client I'm serving really the approved binary on an approved
device?"** — which regulated customers ask first.

CoCo's differentiator is a two-sided attested channel:

- **Device → Gateway.** A local hardened companion binary (descended from
  `d-inference`) holds a non-extractable Secure-Enclave keypair. Its public
  half is enrolled via the customer's existing MDM. On every request it
  signs a fresh challenge; the gateway rejects anything unsigned.
- **Gateway → Device.** The companion fetches `GET /attest`, pins the
  gateway's `MRTD`, and refuses to forward requests to a gateway whose quote
  fails verification. The pinning is part of the companion's signed config.
- **Agent → Companion.** `nono-proxy`'s sandbox (Landlock on Linux,
  Seatbelt on macOS) forces the agent's outbound traffic through the
  companion. The agent cannot reach `api.openai.com` directly; attempting to
  is a visible policy violation.

Net property — the thing we sell: **the agent cannot exfiltrate a
credential, the laptop cannot impersonate an approved client, and the
gateway cannot silently serve a tampered binary.** All three properties are
checkable by a third party.

---

## Near term — ship in 2 weeks

**Goal: Phase 1b complete, externally reproducible.**

- `coco-gateway` running on a Phala TDX CVM with secrets provisioned via
  `phala cvms secrets set`.
- `GET /attest` returns a non-debug TDX DCAP QuoteV4 that verifies offline
  against Intel's PCS.
- GHCR image published by `.github/workflows/docker.yml`.
- `DEPLOY.md` takes an operator from a Phala account to a working gateway
  in under fifteen minutes.
- End-to-end demo: Claude Code on a fresh laptop with only the phantom
  token; `grep` on process tree and `tcpdump` on the laptop prove the real
  `sk-ant-…` never appears locally.

**Success criterion:** one external developer deploys their own instance
from the docs alone, verifies the attestation quote, and sends a completion
through it.

---

## Near term — ship in 2 months

**Goal: a control plane a regulated ten-person team can run in production.**

Four work streams in parallel:

**1. Control plane (inside the TEE).**
- Policy engine: per-token rate limits, daily token budgets, upstream path
  allowlists, request size caps. Hot-reloadable config beside the route
  profile.
- Per-agent phantom tokens: short-lived, scoped, individually revocable.
  Issued by the gateway; bound to a policy bundle at issue time.
- Append-only audit log: timestamp, phantom-token ID, route, upstream path,
  status, bytes in/out, approximate token count. JSON to stdout, streamable
  to Loki, Datadog, or S3.
- `/metrics` (Prometheus): request rate, 4xx/5xx breakdown, upstream
  latency percentiles, budget consumption per token ID.

**2. Local hardened companion (`coco-companion`).**
- Signed macOS binary with Hardened Runtime and notarization.
- Secure-Enclave-held P-256 keypair, non-extractable.
- Fetches `/attest`, pins `MRTD`, refuses to proxy otherwise.
- Presents phantom token + SE-signed challenge on every request to the
  gateway.
- Embeds `nono-proxy` as the local sandbox enforcer.

**3. Mutual-attestation MVP.**
- Gateway accepts and verifies the SE signature.
- Gateway binds the phantom-token issuance to an MDM-enrolled device
  public key at issue time.
- Companion refuses to launch if its own binary digest does not match the
  signed manifest.

**4. Enrollment.**
- One-shot flow: operator signs in via OIDC (Okta/Entra) → gateway issues a
  phantom bundle for device X → MDM pushes the companion binary and config
  → first launch enrolls the SE public key with the gateway.

**Success criterion:** a regulated ten-person team runs twenty agents for a
week, no raw provider key on any laptop, the security lead can answer "who
spent my Anthropic budget yesterday, from which device, under which policy"
from the audit log in under a minute.

---

## Direction — 2 years

**Goal: the default credential layer for regulated agent fleets.**

When a CISO in a regulated industry asks *"how are our agents handling API
keys?"*, the defensible answer is "through an attested credential
mediator, and here is the quote, the audit trail, and the policy bundle
the auditor asked for." CoCo is that answer.

Concretely that means:

- **Substrate portability.** The gateway runs on Phala today; on Azure
  Confidential, GCP Confidential, and AWS Nitro tomorrow. The encrypted
  vault format is portable across substrates; operators pick their TEE
  without re-provisioning secrets.
- **Companion portability.** The macOS companion is the first substrate.
  Windows (Defender Application Control + TPM-backed keys) and Linux
  (`nono-proxy` Landlock enforcement + TPM or SE-style keys where
  available) follow.
- **Issuance tied to corporate identity.** Phantom tokens mint from OIDC
  claims; device binding from MDM; revocation is a single API call that
  propagates in seconds.
- **Delegation.** An agent can hand a narrower phantom to a sub-agent or
  tool call, and the narrowing is enforced by the gateway, not by the
  sub-agent.
- **Signed receipts.** The gateway signs every response summary with an
  attested key, giving customers evidence they can show auditors and
  regulators without trusting us.
- **A hosted offering.** "CoCo Cloud" for teams that want the gateway
  without running the TEE substrate themselves.

The 2-year success criterion: at least two major agent frameworks ship
CoCo-aware defaults, and at least one regulated industry association (a
bank consortium, a healthcare alliance, a law-firm tech committee) cites
attested credential mediation as a recommended control.

---

## Architecture

```
┌──────────────────────────────────┐        ┌────────────────────────────────┐
│  Developer laptop (MDM-managed)  │        │  Phala TDX CVM                 │
│                                  │        │                                │
│  ┌──────────┐    ┌────────────┐  │  mTLS  │  ┌──────────────────────────┐  │
│  │  Agent   │───▶│ coco-      │──┼───────▶│  │  coco-gateway            │  │
│  │ (Claude  │    │ companion  │  │ +phantom│ │  - policy engine         │  │
│  │  Code)   │    │            │  │ +SE sig │ │  - audit log             │  │
│  │          │    │ nono-proxy │  │        │  │  - /attest (TDX QuoteV4) │  │
│  │          │    │ sandbox ───┤  │        │  │  - credential vault      │  │
│  └──────────┘    │ SE keypair │  │        │  └──────────┬───────────────┘  │
│                  │ MRTD pin   │  │        │             │                  │
│                  └────────────┘  │        │             ▼                  │
│                                  │        │         upstream API (TLS)     │
└──────────────────────────────────┘        └────────────────────────────────┘
       ▲                                                   │
       │  OIDC sign-in, MDM enrolls companion              │
       │  and registers SE public key with gateway         │
       └───────────────────────────────────────────────────┘
```

The agent sees only `http://localhost`. The companion sees only the gateway.
The gateway sees only attested companions. Upstream APIs see only the
gateway.

---

## Open problems we are deliberately not pretending to have solved

**Secret provisioning UX.** Today an operator runs `phala cvms secrets set`
from a laptop — meaning the plaintext key touches a general-purpose OS at
least once. The direction we want to pursue: **provision secrets from a
phone using a passkey.** The operator enters the real API key into a mobile
app, the passkey-backed WebAuthn signature authorizes an encrypted envelope
addressed to the TEE's attested public key, and the plaintext never touches
a laptop or a CLI. This needs design work — envelope format, recovery,
rotation, multi-admin quorum, key-rollover — and we will not ship
hand-wavy; but it is the right target for "first-time credential entry"
and a likely 6-to-9-month item.

**Non-macOS companion.** `nono-proxy` already targets Linux (Landlock).
Windows needs Defender Application Control + a TPM-held client key;
Microsoft's tooling here is workable but not plug-and-play. We need one
design partner per platform before committing.

**Revoking a compromised gateway image.** MRTD pinning plus short-lived
secret leases is the answer. The operational playbook (who signs the new
manifest, how fast companions pick it up, how to fail closed) is not yet
written.

**Failure modes of the in-path companion.** The companion becomes a hot-path
dependency. It needs a local health endpoint, a circuit breaker, and a
clearly documented "fail closed" stance — a misbehaving companion must not
silently degrade to "agent talks to upstream directly."

---

## The library path

Two primitives inside this product are reusable, and extracting them is
worth doing — **once we have a second consumer**, not before:

- **`attested-channel`**: a symmetric, substrate-pluggable handshake. Party
  A presents a TDX quote, party B presents an SE/MDA/TPM assertion, both
  sides bind the session key to their attestation. Trait-based substrates
  so a TDX-to-SEV channel works the same as a TDX-to-macOS channel.
- **`credential-mediator`**: phantom-token issuance and validation, route
  store, credential injection, policy hook, audit hook, attestation
  endpoint — today scattered across `nono-proxy` and `coco-gateway`. The
  Vault-core-to-Vault-plugins shape.

These extract cleanly when `coco-gateway`, `coco-companion`, and `nono`
have all shipped and the overlap is obvious. We will not build them as
speculative infrastructure ahead of that point.

The framing for a future standards conversation: **Capability-Substitution
Proxying — sandboxed code holds a phantom capability; an attested mediator
substitutes the real credential on the sandbox boundary.** Applies far
beyond AI agents: cloud CLIs, CI runners, browser extensions, MCP tool
servers. Worth naming now; worth pursuing only after two products prove it.

---

## Non-goals

- **Not a general HTTPS forward proxy.** Only explicitly configured
  upstreams with well-understood credential shapes. No generic `CONNECT`.
- **Not a model router.** No model selection, prompt rewriting, completion
  caching, semantic routing. Keeping the gateway dumb is a feature.
- **Not a replacement for Vault or cloud KMS** for non-agent workloads.
  CoCo is specifically the answer for "agent → external API" traffic.
- **Not a silver bullet for prompt injection.** Holding the credential
  outside the agent limits blast radius; policy and budgets contain a
  misbehaving agent; nothing here makes the agent itself trustworthy.
- **Not a fork of nono.** We integrate with it. Where our work generalizes
  (companion binary, attested-channel library), we contribute back.
- **Not a parallel identity system.** Phantom tokens mint from existing
  corporate identity; device binding from existing MDM. We add a control
  plane for agents, not a new directory for people.
