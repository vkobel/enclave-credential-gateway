# CoCo Credential Gateway — Product Vision

## What this is

A small Rust HTTP proxy you deploy once into a TEE. It holds your real
upstream API keys (OpenAI, Anthropic, GitHub, …) and gives your agents
**phantom tokens** in their place — scoped, revocable credentials that are
worthless outside your gateway. The gateway validates the phantom in
constant time, swaps in the real key, and forwards the request. Real keys
never leave the enclave; your laptop, your CI, your phone all hold
phantoms.

The core insight: **credentials are infrastructure, not agent state.**

---

## Who v1 is for

**You. One technical AI user with paid API keys and several places that
need to use them** — Claude Code on a laptop, an agent on a desktop, a CI
job, maybe a phone shortcut. You are tired of:

- Pasting `sk-…` keys into config files on every machine.
- Not knowing which agent burned through your monthly budget.
- Rotating one leaked key in seven places.
- Trusting every tool you install with full provider access.

You are not a regulated enterprise. You don't need MDM, OIDC, or
hardware-attested clients. You trust your own laptop. You want a personal
key vault that lives in a place even *you* can't accidentally `cat`.

(The enterprise story — mutual attestation, MDM-bound devices, signed
receipts — is real, and it's the long-term commercial direction. It is
deliberately out of scope for v1. See the roadmap.)

---

## The v1 promise

> *Deploy once. Add your real keys once. Mint a phantom per agent.
> Point every agent at the gateway. Watch the audit log to see who
> spent what. Revoke a phantom in one command when you're done.*

Single user. Multiple clients. One vault. Verifiable hardware isolation.

---

## v1 — definition of done

A user with no Rust knowledge can complete this flow in **under 30
minutes**, end to end:

1. **Deploy.** One command (`coco deploy phala` or `docker compose up`)
   stands up the gateway on Phala TDX CVM (or any Docker host). Deploy
   prints an `admin token` once.
2. **Verify the binary.** `coco verify <gateway-url>` fetches `GET /attest`,
   checks the TDX QuoteV4 against Intel's PCS, asserts no debug bit, and
   prints the `MRTD` so the user can pin it.
3. **Add real credentials.** `coco creds add openai sk-…` and
   `coco creds add anthropic sk-ant-…`. Stored encrypted at rest, only
   readable inside the TEE.
4. **Mint phantoms per client.**
   ```
   coco token create --name laptop-claude-code --routes anthropic --budget 100k-tokens/day
   coco token create --name ci-runner          --routes openai,anthropic --expires 30d
   coco token create --name phone-shortcut     --routes anthropic --budget 10k-tokens/day
   ```
5. **Use them.** Each agent points at the gateway URL with its phantom in
   place of the real key. Existing SDKs work with no code changes (the
   gateway accepts the phantom in the same header the SDK already sends).
6. **Watch.** `coco audit tail` streams every request: which phantom, which
   route, status, bytes, approximate token count.
7. **Revoke.** `coco token revoke laptop-claude-code` cuts that phantom in
   under a second. The other phantoms keep working.

### What's in v1

- TDX CVM deployment (Phase 1b, in flight).
- `GET /attest` returning verified non-debug TDX QuoteV4.
- Encrypted credential store inside the TEE (sealed by Phala secret
  injection or by a unsealing key derived inside the enclave).
- Phantom token registry (name, routes allowlist, budget caps, expiry,
  status), persisted encrypted at rest.
- Constant-time phantom validation, multi-source credential injection
  (already done in `coco-gateway`).
- Per-token policy: routes allowlist, daily request count cap, daily
  approximate-token-count cap for LLM routes, hard expiry.
- Append-only structured audit log, queryable via admin API, optionally
  streamed to a file or S3.
- Admin API (`/admin/*`) authenticated by a single admin token printed at
  deploy.
- `coco` CLI: `deploy`, `verify`, `creds {add|rotate|rm|ls}`,
  `token {create|revoke|ls}`, `audit {tail|grep}`.
- One-page `DEPLOY.md` and `USING.md`.

### What is explicitly NOT in v1

- **No mutual attestation, no client-side hardened binary.** You trust your
  own devices; the phantom token is the client identity. (Future.)
- **No web UI.** CLI is enough for the target user; a UI is a v1.x.
- **No multi-tenant.** One admin token, one operator. (Future.)
- **No OIDC, no MDM, no SSO.** (Future, enterprise.)
- **No mobile/passkey provisioning.** (Identified as a v1.x delight feature
  — see open problems.)
- **No model routing, no caching, no prompt rewriting.** Out of scope
  forever — different product.

---

## Architecture (v1)

```
┌─────────────────┐                     ┌──────────────────────────────────┐
│ Laptop / CI /   │                     │ Phala TDX CVM                    │
│ phone / etc.    │                     │                                  │
│                 │   phantom token     │  ┌────────────────────────────┐  │
│   ┌──────────┐  │  ───────────────▶   │  │ coco-gateway               │  │
│   │ agent    │──┼─────────────────────┼─▶│  - phantom registry        │  │
│   │ (any SDK)│  │   over TLS          │  │  - per-token policy        │  │
│   └──────────┘  │                     │  │  - audit log               │  │
│                 │                     │  │  - encrypted cred store    │  │
│   ┌──────────┐  │                     │  │  - GET /attest (TDX quote) │  │
│   │ coco CLI │──┼─────────────────────┼─▶│  - /admin (admin token)    │  │
│   └──────────┘  │   admin token       │  └─────────────┬──────────────┘  │
└─────────────────┘                     │                │                 │
                                        │                ▼ (real key)      │
                                        │         upstream API (TLS)       │
                                        └──────────────────────────────────┘
```

The agent sees only the gateway URL and a phantom. The CLI talks to the
same gateway with an admin token. The real credential exists only inside
the enclave.

---

## The 4-week path to v1

Anchored on what's actually in the repo today.

**Week 1 — Phase 1b ships.**
`/attest` returns a verified non-debug TDX QuoteV4. GHCR image published.
`DEPLOY.md` walks an operator from "I have a Phala account" to a working
gateway in under 15 minutes. End-to-end demo: Claude Code + phantom token,
real key never on the laptop.

**Week 2 — Phantom token registry.**
Replace the single `COCO_PHANTOM_TOKEN` env var with an encrypted registry
inside the TEE. Add the admin token, the `/admin/tokens` API, and the
`coco token {create|revoke|ls}` CLI subcommands. Existing single-token
behavior is preserved as a `--legacy-token` startup flag for one release.

**Week 3 — Policy + audit log.**
Per-token route allowlist, daily request cap, daily token-count cap (for
LLM routes; approximate from request/response sizes). Append-only audit
log to an encrypted on-disk volume, plus optional S3 sink. `coco audit
tail` and `coco audit grep`.

**Week 4 — Polish.**
`coco deploy phala` one-shot deploy helper. `coco verify` for attestation.
`coco creds {add|rotate|rm|ls}` against a sealed credential store.
`USING.md` with copy-paste recipes for Claude Code, OpenAI Python SDK,
GitHub CLI. End-to-end test that exercises the full flow from a fresh
machine.

**Week 4, end:** the v1 promise is true. Ship it.

---

## Tentative roadmap after v1

Each step is a direction, not a commitment. Each is justified by what v1
*didn't* solve.

**v1.x — delight & ergonomics.**
- **Mobile provisioning via passkey.** A small companion mobile app where
  the operator pastes a real upstream key once, signs an envelope with a
  passkey (WebAuthn), and the envelope is encrypted to the TEE's attested
  public key. The plaintext never touches a laptop or CLI. This is the
  right experience for first-time credential entry; we will not ship
  hand-wavy crypto for it.
- **Minimal admin web UI** for non-CLI users.
- **Profile presets** for the top ten upstreams (currently you copy-paste
  from `examples/profile.json`).

**v2 — small teams.**
- Multi-operator: more than one admin, role separation (admin / read-only /
  audit).
- Per-operator API tokens replacing the single admin token.
- Audit log with operator attribution.
- Hosted "CoCo Cloud" option for users who don't want to run a CVM
  themselves.

**v3 — enterprise / regulated.**
- Mutual attestation. A hardened local companion (Secure-Enclave-backed
  client identity, descended from `d-inference`) that authenticates the
  device to the gateway, with the gateway's `MRTD` pinned in the
  companion's signed config.
- OIDC issuance (Okta, Entra) bound to MDM-enrolled device identity.
- Signed per-call receipts under an attested gateway key.
- Substrate portability beyond Phala (Azure Confidential, GCP Confidential,
  AWS Nitro).
- Compliance posture: SOC2-friendly audit format, exportable evidence
  bundles.

The v3 work is the eventual commercial direction. v1 exists to prove the
model and build a base of users who become the design partners for v2 and
the references for v3.

---

## Open problems we are not pretending to have solved

- **First-time credential entry UX.** Today the operator pastes real keys
  into a CLI. The passkey/mobile envelope flow above is the target;
  envelope format, recovery path, and rotation still need design.
- **Sealed credential storage at rest.** v1 ships with credentials sealed
  by Phala's secret injection (the simple path). A portable, substrate-
  agnostic vault format is a v2 item.
- **Audit-log integrity.** v1 logs are append-only on an encrypted volume.
  Hash-chained, tamper-evident logs (and signed receipts) are v3.

---

## Non-goals (forever)

- **Not a general HTTPS forward proxy.** Only explicitly configured
  upstreams. No `CONNECT`.
- **Not a model router.** No model selection, prompt rewriting, completion
  caching, semantic routing.
- **Not a Vault replacement** for non-agent workloads. CoCo is specifically
  for "agent → external API" traffic.
- **Not a silver bullet for prompt injection.** Holding the credential
  outside the agent limits blast radius; policy and budgets contain a
  misbehaving agent. Nothing here makes the agent itself trustworthy.
- **Not a parallel identity system.** v3 will mint phantoms from existing
  corporate identity, not invent a new one.
