# CoCo Credential Gateway - Vision

> Status: this is the product vision and target architecture. The current repository has a working proxy, phantom token registry, and CLI activation flow. TDX attestation, sealed credential storage, audit logging, and several route profiles described here are roadmap work.

## What this is

The target is a personal, hardware-attested credential hub that you deploy
once and every agent you run connects to. It holds API keys, bot tokens,
and service credentials that today live in `.env` files, config files, or
shell exports. Your agents never receive those values. They receive
**phantom tokens** - scoped, revocable identifiers that are worthless
outside your gateway.

The working implementation already validates phantom tokens, checks route
scope, injects real upstream credentials server-side, and forwards the
result. The target TEE implementation moves that credential boundary inside
Intel TDX so the real key never leaves the enclave.

CoCo is meant to be to AI agents what a hardware password manager is to browsers -
except the credentials never leave the device even to fill a form,
because CoCo fills the form itself.

The core insight: **credentials are infrastructure, not agent state.**

### Why not a local proxy?

Local proxies (OneCLI, AgentSecrets) are a meaningful
security step up from `.env` files. They protect credentials from the agent
process. CoCo's target TEE architecture protects credentials from
*everyone* - including the infrastructure operator. And unlike any local
proxy, CoCo is a single network-accessible hub: one deployment, every
agent.

| | Local proxy | CoCo target |
|---|---|---|
| Agent can’t read the key | ✅ | ✅ |
| Operator can’t read the key | ❌ host access = full access | target: enclave boundary |
| Works from any device / CI | ❌ local only | ✅ network-accessible |
| One change updates all agents | ❌ restart every proxy | ✅ gateway is the source of truth |
| Cryptographically verifiable binary | ❌ | target: TDX attestation + MRTD |
| Audit trail is tamper-resistant | ❌ process can lie | target: log produced inside attested binary |

A local proxy is a first step. CoCo is the destination.

Technically, the target architecture is a TEE-backed RFC 8693 Security
Token Service. The phantom token is the `subject_token`; the TEE is the
STS; the injected credential, or a short-lived derivative of it, is the
output `access_token`. The credential participates in the live HTTP request
inside a hardware boundary. This is closer to an HSM than to a vault: an
HSM signs data on your behalf without exposing the key; CoCo authenticates
HTTP requests on your behalf without exposing the credential.

---

## Who v1 is for

**You. One technical AI user running multiple agents across multiple
devices, tired of managing credentials the way it’s done today.**

The problem looks like this. You have a `.env` on your laptop for Claude
Code. A separate config on your desktop. An `OPENAI_API_KEY` in your CI
secrets. A `GITHUB_TOKEN` and `TELEGRAM_BOT_TOKEN` in an n8n installation
somewhere. Your agents don’t just call LLMs: they open PRs with `gh`,
send notifications to Telegram, update Linear issues, post to Slack, edit
Notion pages. Every one of those is another real credential sitting on
every host you run an agent from.

Every time you rotate a key you hunt down every place it lives. Every time
you add a new agent you copy credentials to one more location. Every agent
holds full, unrestricted, permanent access to every key it was given.

CoCo collapses this: deploy once, add your credentials once, point every
agent at the same gateway. You are tired of:

- Credentials scattered across `.env` files, config files, shell exports,
  and CI secret panels on multiple machines.
- Rotating one leaked key in seven places, missing two, finding out later.
- Having no idea which agent called which API at 3am.
- Every agent holding full unrestricted permanent access to every key.
- Trusting every tool you install not to read your credential files.

You are not a regulated enterprise. You don’t need MDM, OIDC, or
hardware-attested clients. You trust your own laptop. You want a personal
credential hub that is verifiably isolated — a place even *you* can’t
accidentally `cat` — and that all your agents connect to instead of
holding their own copies.

(The enterprise story - mutual attestation, MDM-bound devices, signed
receipts - is real and is the long-term commercial direction. It is
deliberately out of scope for v1. See the roadmap.)

---

## The v1 Promise

> *Deploy once. Add every credential once - LLM keys, GitHub tokens,
> Telegram bots, all of it. Mint a phantom per agent. Point every agent
> at the gateway. One credential change propagates everywhere instantly.
> Watch the audit log to see who called what and when. Revoke any agent
> in one command.*

Single user. Every agent. Every credential. One hub. Verifiable hardware isolation.

### What “every credential” means

CoCo is not only for LLM APIs. It is for any credential that today lives
in an agent’s environment and is consumed over HTTP:

- **LLM APIs:** OpenAI, Anthropic, Mistral, Groq, …
- **Developer tools:** GitHub PAT, GitLab token, npm auth, …
- **Messaging:** Telegram Bot API token, Slack bot token, Discord, …
- **SaaS tools:** Stripe, Sendgrid, Notion, Linear, Airtable, …
- **Self-hosted services:** anything behind Bearer or a custom header

If it ends up in a `.env` and is consumed over HTTP, it belongs here.
Agents stop holding credentials. They hold phantoms.

### The central-hub model

```
  TODAY
  ─────
  claude-code (laptop)      -> OPENAI_KEY from ~/.env
  ci-runner                 -> OPENAI_KEY from GitHub Secrets
  telegram-bot (VPS)        -> TELEGRAM_TOKEN from /etc/systemd/...
  n8n workflow              -> GITHUB_TOKEN from n8n credential store
  -> 4 locations. Key rotated once = 3 missed. Zero audit trail.

  WITH COCO
  ─────────
  claude-code (laptop)   phantom ccgw_a1...  ─┐
  ci-runner              phantom ccgw_b2...  ─┤
  telegram-bot (VPS)     phantom ccgw_c3...  ─┼─▶  CoCo TEE gateway  ─▶  upstream APIs
  n8n workflow           phantom ccgw_d4...  ─┘
  -> 1 location. Key rotated once = propagates immediately to all agents.
    Full per-agent audit trail.
```

Credentials live in exactly one place. Phantoms are worthless outside the
gateway. Rotation is a single command.

---

## v1 Definition of Done

A user with no Rust knowledge can complete this flow in **under 30
minutes**, end to end:

1. **Deploy.** One command (`coco deploy phala` or `docker compose up`)
   stands up the gateway on Phala TDX CVM (or any Docker host). Deploy
   prints an `admin token` once.
2. **Verify the binary.** `coco verify <gateway-url>` fetches `GET /attest`,
   checks the TDX QuoteV4 against Intel's PCS, asserts no debug bit, and
   prints the `MRTD` so the user can pin it.
3. **Add real credentials.** `coco creds add openai sk-...` and
   `coco creds add anthropic sk-ant-...`. Stored encrypted at rest, only
   readable inside the TEE.
4. **Mint phantoms per client.**
   ```
   coco token create --name laptop-claude-code --routes anthropic,github
   coco token create --name ci-runner          --routes openai,anthropic,github  --expires 30d
   coco token create --name phone-shortcut     --routes telegram
   ```
5. **Use them.** Each agent points at the gateway URL with its phantom in
   place of the real key. Existing SDKs work with no code changes (the
   gateway accepts the phantom in the same header the SDK already sends).
6. **Watch.** `coco audit tail` streams every request: which phantom, which
   route, status, bytes, approximate token count.
7. **Revoke.** `coco token revoke laptop-claude-code` cuts that phantom in
   under a second. The other phantoms keep working.

### Working in the repo today

- Constant-time phantom validation and server-side credential injection (`coco-gateway`).
- Named token registry with Blake3-hashed tokens. Admin API: `POST/GET/DELETE /admin/tokens`.
- Scope enforcement: per-token route allowlist, 403 before credential resolution.
- Route profile schema with route-owned aliases and prefix-based credential sources.
- Shipped profiles: **OpenAI, Anthropic, GitHub**.
- Caddy TLS termination and `GET /health`.
- `coco` CLI: `activate`, `token {create|revoke|ls}`, and `git-credential`.

### Still required for v1

- TDX CVM deployment.
- `GET /attest` returning a non-debug TDX QuoteV4.
- Reproducible release artifacts and MRTD publication.
- Encrypted credential store inside the TEE.
- Per-token hard expiry and finer endpoint policy.
- Append-only structured audit log.
- Additional route profiles: Groq, ElevenLabs, Ollama, Telegram, Together.
- `coco` CLI: `deploy`, `verify`, `creds {add|rotate|rm|ls}`, `audit {tail|grep}`.
- One-page `DEPLOY.md`.

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

## Target Architecture (v1)

```
 ┌──────────────────────────────────────────────┐
 │  Your devices / infrastructure               │
 │                                              │
 │  claude-code (laptop)  phantom ccgw_a1…  ─┐  │
 │  ci-runner             phantom ccgw_b2…  ─┤  │
 │  telegram-bot (VPS)    phantom ccgw_c3…  ─┤  │
 │  n8n workflow          phantom ccgw_d4…  ─┘  │
 │                               │              │
 │  coco CLI  ──admin-token──────┤              │
 └──────────────────────────────┼──────────────┘
                                │ TLS
                                ▼
           ┌────────────────────────────────────┐
           │ Phala TDX CVM  (hardware boundary) │
           │                                    │
           │  coco-gateway                      │
           │  ├─ phantom registry (enc)         │
           │  ├─ credential store  (enc)        │
           │  │    openai    -> sk-proj-...     │
           │  │    github    -> ghp_...         │
           │  │    telegram  -> 7312...:AAH...  │
           │  │    stripe    -> sk_live_...     │
           │  ├─ per-token policy               │
           │  ├─ audit log                      │
           │  └─ GET /attest  (TDX QuoteV4)     │
           └──────────────┬─────────────────────┘
                          │ real credential injected here
              ┌───────────┼───────────┐
              ▼           ▼           ▼
         api.openai   api.github  api.telegram
```

In the target v1 architecture, every agent connects to the same gateway
with a different phantom. Credentials live in one place. Rotation
propagates immediately to all agents. The CLI is the only management
surface. Real keys exist only inside the enclave.

## How agents connect

Two integration modes. The right choice depends on what the agent framework exposes:

**Option A - Base URL rewrite** (LLM APIs and any SDK with a configurable base URL)

Change `base_url` to point at the gateway. No code changes - the phantom
goes in the same `Authorization` header the SDK already sends.

```bash
# OpenAI Python SDK
client = OpenAI(base_url="https://gw.example/openai", api_key="ccgw_a1...")

# Claude Code
ANTHROPIC_BASE_URL=https://gw.example/anthropic ANTHROPIC_API_KEY=ccgw_a1... claude
```

**Option B - `HTTPS_PROXY`** (future proxy mode for HTTP tools with no configurable base URL)

```bash
export HTTPS_PROXY=https://ccgw_a1...@gw.example
# Every subsequent HTTP call goes through CoCo.
# The gateway strips the phantom from proxy credentials and injects the real credential.
```

Option A is the current integration path for shipped LLM routes and tool
adapters. Option B (`CONNECT` proxy mode) is future work.

---

## The Path to v1

Anchored on what's actually in the repo today.

**Phase 1c - done.**
Named token registry with admin API (`POST/GET/DELETE /admin/tokens`),
scope enforcement, Blake3 hashing at rest. Shipped profiles for OpenAI,
Anthropic, and GitHub. Caddy TLS. Local `coco` CLI with activation for
Claude Code, Codex, and `gh`, plus `token {create|ls|revoke}` subcommands.
Backwards-compatible with
single `COCO_PHANTOM_TOKEN` env var.

**Phase 1b - next.**
`/attest` returns a verified non-debug TDX QuoteV4. GHCR image published.
`DEPLOY.md` walks an operator from "I have a Phala account" to a working
gateway in under 15 minutes. End-to-end demo: Claude Code + phantom token,
real key never on the client host.

**Phase 2 - Policy + audit log + more profiles.**
Per-token route allowlist, per-route `endpoint_rules` (method + path), hard expiry.
Append-only audit log to an encrypted on-disk volume, plus optional S3 sink.
`coco audit tail`, `coco audit grep`, and additional route profiles.

**Phase 3 - Polish.**
`coco deploy phala` one-shot deploy helper. `coco verify` for attestation.
`coco creds {add|rotate|rm|ls}` against a sealed credential store.
`USING.md` with copy-paste recipes for Claude Code, OpenAI Python SDK,
GitHub CLI, and a Telegram bot. End-to-end test that exercises the full
flow from a fresh machine.

**Phase 3, end:** the v1 promise is true. Ship it.

---

## Tentative roadmap after v1

Each step is a direction, not a commitment. Each is justified by what v1
*didn't* solve.

**v1.x - delight & ergonomics.**
- **Mobile provisioning via passkey.** A small companion mobile app where
  the operator pastes a real upstream key once, signs an envelope with a
  passkey (WebAuthn), and the envelope is encrypted to the TEE's attested
  public key. The plaintext never touches a laptop or CLI. This is the
  right experience for first-time credential entry; we will not ship
  under-specified crypto for it.
- **Minimal admin web UI** for non-CLI users.
- **Route presets beyond the v1 set** (Cloudflare, Stripe, Pushover,
  AWS SigV4, Google OAuth, …). v1 ships a compiled built-in route
  manifest; additional route extension needs an explicit design.

**v2 - small teams.**
- Multi-operator: more than one admin, role separation (admin / read-only /
  audit).
- Per-operator API tokens replacing the single admin token.
- Audit log with operator attribution.
- Hosted "CoCo Cloud" option for users who don't want to run a CVM
  themselves.

**v3 - enterprise / regulated.**
- Mutual attestation. A hardened local companion (Secure-Enclave-backed
  client identity) that authenticates the device to the gateway, with the
  gateway's `MRTD` pinned in the companion's signed config.
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
- **Sealed credential storage at rest.** The target v1 path is credentials
  sealed by Phala's secret injection or an enclave-derived key. A portable,
  substrate-agnostic vault format is a v2 item.
- **Audit-log integrity.** v1 logs are append-only on an encrypted volume.
  Hash-chained, tamper-evident logs (and signed receipts) are v3.

---

## Non-goals (forever)

- **Not a model router.** No model selection, prompt rewriting, completion
  caching, semantic routing.
- **Not a Vault replacement** for non-agent workloads. CoCo is specifically
  for “agent → external API” HTTP traffic.
- **Not a silver bullet for prompt injection.** Holding the credential
  outside the agent limits blast radius; route scoping contains a
  misbehaving agent. Nothing here makes the agent itself trustworthy.
- **Not a parallel identity system.** v3 will mint phantoms from existing
  corporate identity, not invent a new one.
