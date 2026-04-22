# CoCo Credential Gateway — Product Vision

## What this is

A personal, hardware-attested credential hub that you deploy once and every
agent you run connects to. It holds every API key, bot token, and service
credential you own — GitHub, Telegram, Stripe, OpenAI, Anthropic, anything
that today lives in a `.env`, a config file, or a shell export. Your agents
never receive those values. They receive **phantom tokens** — scoped,
revocable identifiers that are worthless outside your gateway. The gateway
validates the phantom, injects the real credential into the live HTTP
request inside the hardware boundary, and forwards the result. The real key
never leaves the enclave.

CoCo is to AI agents what a hardware password manager is to browsers —
except the credentials never leave the device even to fill a form,
because CoCo fills the form itself.

The core insight: **credentials are infrastructure, not agent state.**

### Why not a local proxy?

Local proxies (OneCLI, AgentSecrets, nono's proxy mode) are a meaningful
security step up from `.env` files. They protect credentials from the agent
process. CoCo protects credentials from *everyone* — including the
infrastructure operator. And unlike any local proxy, CoCo is a single
network-accessible hub: one deployment, every agent.

| | Local proxy | CoCo (TEE) |
|---|---|---|
| Agent can’t read the key | ✅ | ✅ |
| Operator can’t read the key | ❌ host access = full access | ✅ enclave boundary |
| Works from any device / CI | ❌ local only | ✅ network-accessible |
| One change updates all agents | ❌ restart every proxy | ✅ gateway is the source of truth |
| Cryptographically verifiable binary | ❌ | ✅ TDX attestation + MRTD |
| Audit trail is tamper-resistant | ❌ process can lie | ✅ log produced inside attested binary |

A local proxy is a baby step. CoCo is the destination.

Technically: CoCo is a TEE-backed RFC 8693 Security Token Service. The
phantom token is the `subject_token`; the TEE is the STS; the injected
credential — or a short-lived derivative of it — is the output
`access_token`. The credential participates in the live HTTP request inside
a hardware boundary. This is closer to an HSM than to a vault: an HSM signs
data on your behalf without exposing the key; CoCo authenticates HTTP
requests on your behalf without exposing the credential.

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

(The enterprise story — mutual attestation, MDM-bound devices, signed
receipts — is real and is the long-term commercial direction. It is
deliberately out of scope for v1. See the roadmap.)

---

## The v1 promise

> *Deploy once. Add every credential once — LLM keys, GitHub tokens,
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
  claude-code (laptop)      → OPENAI_KEY from ~/.env
  ci-runner                 → OPENAI_KEY from GitHub Secrets
  telegram-bot (VPS)        → TELEGRAM_TOKEN from /etc/systemd/…
  n8n workflow              → GITHUB_TOKEN from n8n credential store
  → 4 locations. Key rotated once = 3 missed. Zero audit trail.

  WITH COCO
  ─────────
  claude-code (laptop)   phantom ccgw_a1…  ─┐
  ci-runner              phantom ccgw_b2…  ─┤
  telegram-bot (VPS)     phantom ccgw_c3…  ─┼─▶  CoCo TEE gateway  ─▶  upstream APIs
  n8n workflow           phantom ccgw_d4…  ─┘
  → 1 location. Key rotated once = propagates immediately to all agents.
    Full per-agent audit trail.
```

Credentials live in exactly one place. Phantoms are worthless outside the
gateway. Rotation is a single command.

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
   coco token create --name laptop-claude-code --routes anthropic,github         --budget 100k-tokens/day
   coco token create --name ci-runner          --routes openai,anthropic,github  --expires 30d
   coco token create --name phone-shortcut     --routes telegram                 --budget 1k-requests/day
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
- Route profile schema covering every upstream shape an agent hits:
  `inject_mode` ∈ {`header`, `url_path`, `query_param`, `basic_auth`},
  prefix-based `inject_overrides` (e.g. Anthropic API key vs Claude Code
  OAuth token), and per-route `endpoint_rules` (method + path allowlist).
- Curated route profiles shipped with v1 for the upstreams a solo
  operator actually uses: **OpenAI, Anthropic, GitHub, Telegram, Slack,
  Linear, Notion**. Telegram drives the `url_path` case (bot token lives
  in the URL: `/bot<TOKEN>/<method>`); the rest are header-mode.
- Per-token policy: routes allowlist, per-route `endpoint_rules`
  (method + path), daily request count cap, daily approximate-token-count
  cap for LLM routes, hard expiry.
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
           │  │    openai    → sk-proj-…        │
           │  │    github    → ghp_…            │
           │  │    telegram  → 7312…:AAH…       │
           │  │    stripe    → sk_live_…        │
           │  ├─ per-token policy               │
           │  ├─ audit log                      │
           │  └─ GET /attest  (TDX QuoteV4)     │
           └──────────────┬─────────────────────┘
                          │ real credential injected here
              ┌───────────┼───────────┐
              ▼           ▼           ▼
         api.openai   api.github  api.telegram
```

Every agent connects to the same gateway with a different phantom.
Credentials live in one place. Rotation propagates immediately to all agents.
The CLI is the only management surface. Real keys exist only inside the enclave.

## How agents connect

Two integration modes. The right choice depends on what the agent framework exposes:

**Option A — Base URL rewrite** (LLM APIs and any SDK with a configurable base URL)

Change `base_url` to point at the gateway. No code changes — the phantom
goes in the same `Authorization` header the SDK already sends.

```bash
# OpenAI Python SDK
client = OpenAI(base_url="https://gw.example/openai", api_key="ccgw_a1…")

# Claude Code
ANTHROPIC_BASE_URL=https://gw.example/anthropic ANTHROPIC_API_KEY=ccgw_a1… claude
```

**Option B — `HTTPS_PROXY`** (any HTTP tool with no configurable base URL: `gh`, Telegram SDK, curl, shell scripts, n8n HTTP nodes)

```bash
export HTTPS_PROXY=https://ccgw_a1…@gw.example
# Every subsequent HTTP call — gh, curl, the Telegram library, anything — goes through CoCo.
# The gateway strips the phantom from proxy credentials and injects the real credential.
```

Option A ships in v1. Option B (`CONNECT` proxy mode) is the first v1.x
priority: it unlocks the non-LLM tool case (`gh`, Telegram, etc.) with
zero per-agent configuration and is the natural path for agents that
don’t expose a base URL setting.

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

**Week 3 — Policy + audit log + non-LLM tool profiles.**
Per-token route allowlist, per-route `endpoint_rules` (method + path),
daily request cap, daily token-count cap (for LLM routes; approximate
from request/response sizes). Extend the profile schema with
`inject_mode` (header / url_path / query_param / basic_auth) so the
gateway can front Telegram (url_path) alongside header-based upstreams.
Ship the curated GitHub / Telegram / Slack / Linear / Notion profiles.
Append-only audit log to an encrypted on-disk volume, plus optional S3
sink. `coco audit tail` and `coco audit grep`.

**Week 4 — Polish.**
`coco deploy phala` one-shot deploy helper. `coco verify` for attestation.
`coco creds {add|rotate|rm|ls}` against a sealed credential store.
`USING.md` with copy-paste recipes for Claude Code, OpenAI Python SDK,
GitHub CLI, and a Telegram bot. End-to-end test that exercises the full
flow from a fresh machine.

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
- **Profile presets beyond the v1 seven** (Cloudflare, Stripe, Pushover,
  AWS SigV4, Google OAuth, …). v1 ships OpenAI, Anthropic, GitHub,
  Telegram, Slack, Linear, Notion; anything else is still a copy-paste
  from `examples/profile.json`.

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

## Relationship to Nono

CoCo's route-profile JSON schema is lifted from [`nono`][nono]'s
`custom_credentials` shape: `upstream`, `credential_key`, `inject_mode`,
`inject_header`, `credential_format`, `inject_overrides`, `endpoint_rules`.
Nono proved that shape covers the upstreams real agents hit (LLMs,
GitHub, Telegram, Slack, …), so v1 reuses it rather than reinventing it.

What v1 takes from Nono: **the JSON schema**, as a vendored format.

What v1 does **not** take: the `nono` Rust crate as a runtime dependency.
Nono's library is a client-side sandbox primitive (Landlock on Linux,
Seatbelt on macOS) bundled with filesystem rules, hooks, keystore
integration, and rollback — none of which apply inside a TDX gateway.
Pulling the crate in would drag along OS-specific enforcement code for a
schema we can parse in ~100 lines.

Practically: the `nono/` submodule stays as reference, `coco-gateway`
owns its own parser, and the two schemas are kept schema-compatible on
the fields they share. Profiles authored for nono's reverse-proxy mode
should load in CoCo with minimal edits, and vice versa.

[nono]: https://github.com/always-further/nono

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

- **Not a model router.** No model selection, prompt rewriting, completion
  caching, semantic routing.
- **Not a Vault replacement** for non-agent workloads. CoCo is specifically
  for “agent → external API” HTTP traffic.
- **Not a silver bullet for prompt injection.** Holding the credential
  outside the agent limits blast radius; policy and budgets contain a
  misbehaving agent. Nothing here makes the agent itself trustworthy.
- **Not a parallel identity system.** v3 will mint phantoms from existing
  corporate identity, not invent a new one.
