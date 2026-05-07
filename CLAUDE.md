# CoCo Credential Gateway — Agent Guide

## What This Project Is

A TEE-backed credential gateway for AI agents. Clients use scoped phantom tokens (`ccgw_...`) instead of real API keys. The gateway validates the phantom inside a hardware trust boundary (Intel TDX), injects the real credential into the live HTTP request, and forwards upstream. Real keys never leave the enclave.

The core insight: **credentials are infrastructure, not agent state.**

## Repository Layout

```
crates/
  coco-gateway/   — Axum HTTP gateway (proxy, auth, registry, admin API)
  coco-cli/       — CLI for token management and shell activation
profiles/
  routes/*.yaml   — Built-in route definitions (embedded at build time)
  tools/*.yaml    — Tool-specific activation adapters (gh, claude-code, codex)
docs/
  USING.md        — Per-tool setup guide (current state)
  TEE-SECURITY.md — TEE threat model and attestation design
spec/
  vision.md       — Product vision and long-term roadmap
  roadmap.md      — Implementation phases and task list (start here for what's next)
scripts/
  test-e2e.sh     — Docker-backed end-to-end test suite
```

## Where to Start

- **Understanding the project:** read `spec/vision.md`
- **What needs to be built next:** read `spec/roadmap.md` — phase 1b (TDX attestation) is the current priority
- **How to set up a tool:** read `docs/USING.md`
- **TEE security design:** read `docs/TEE-SECURITY.md`
- **Route/tool profile format:** read `profiles/README.md`

## Development Rules

- Run `cargo fmt --check` and `cargo test --workspace` before any commit.
- Run `./scripts/test-e2e.sh` when touching gateway routing, auth, registry, Docker config, CLI activation, or README test instructions.
- Keep route and tool behavior consistent across `profiles/routes`, `profiles/tools`, CLI adapter output, README route table, and e2e tests.
- Do not add dependencies without a clear reason. Prefer small, reversible diffs.
- Prefer `coco activate` in docs and scripts. Do not duplicate long setup guides in README — link to `docs/USING.md` instead.

## Key Invariants

- Token validation is constant-time (Blake3 hash compare via `auth.rs`). Never use `==` on raw token values.
- Scope enforcement happens before credential resolution. A 403 must never touch the credential store.
- The admin API (`/admin/...`) requires `Authorization: Bearer <COCO_ADMIN_TOKEN>` validated constant-time.
- `COCO_PHANTOM_TOKEN` env var is a legacy single-token fallback. The registry path is preferred for all new work.
- Routes are embedded at build time from `profiles/routes/*.yaml` — there is no runtime profile loading.
