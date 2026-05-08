# CoCo Credential Gateway - Agent Guide

## What This Project Is

CoCo is early work toward a TEE-backed credential gateway for AI agents. The current repo contains a working Rust gateway/proxy, phantom token registry, and CLI activation flow. The TEE trust boundary, attestation endpoint, sealed credential store, and verification flow are roadmap work.

Clients use scoped phantom tokens (`ccgw_...`) instead of real API keys. The gateway validates the phantom, checks route scope, removes the client credential, injects the real server-side credential, and forwards upstream.

The core idea: **credentials are infrastructure, not agent state.**

## Repository Layout

```text
crates/
  coco-gateway/   - Axum HTTP gateway: proxy, auth, registry, admin API
  coco-cli/       - CLI for token management and shell/tool activation
profiles/
  routes/*.yaml   - Built-in route definitions, embedded at build time
  tools/*.yaml    - Tool-specific activation adapters
docs/
  USING.md        - Per-tool setup guide for the current implementation
  TEE-SECURITY.md - Target TEE threat model and security requirements
spec/
  vision.md       - Product vision and long-term direction
  roadmap.md      - Current status and next implementation milestones
scripts/
  test-e2e.sh     - Docker-backed end-to-end test suite
```

## Where to Start

- **Understand the project:** read `spec/vision.md`.
- **See current status and next work:** read `spec/roadmap.md`.
- **Set up a tool:** read `docs/USING.md`.
- **Review the TEE target:** read `docs/TEE-SECURITY.md`.
- **Change route/tool profiles:** read `profiles/README.md`.

## Development Rules

- Run `cargo fmt --check` and `cargo test --workspace` before code commits.
- Run `./scripts/test-e2e.sh` when touching gateway routing, auth, registry, Docker config, CLI activation, or README test instructions.
- Keep route and tool behavior consistent across `profiles/routes`, `profiles/tools`, CLI adapter output, README route table, and e2e tests.
- Do not add dependencies without a clear reason. Prefer small, reversible diffs.
- Prefer `coco activate` in docs and scripts. Keep long setup guides in `docs/USING.md`, not README.

## Key Invariants

- Token validation uses constant-time comparison. Never compare raw token values with `==`.
- Scope enforcement happens before credential resolution. A 403 must not touch upstream credentials.
- The admin API (`/admin/...`) requires `Authorization: Bearer <COCO_ADMIN_TOKEN>` and validates it constant-time.
- `COCO_PHANTOM_TOKEN` is a legacy single-token fallback. The registry path is preferred for new work.
- Current shipped route profiles are `openai`, `anthropic`, and `github`.
- Routes are embedded at build time from `profiles/routes/*.yaml`; there is no runtime profile loading.
- TDX attestation, sealed storage, audit log, and `coco verify` are planned features, not current behavior.
