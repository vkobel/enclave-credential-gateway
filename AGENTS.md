# CoCo Credential Gateway - Repository Guidelines

> For agent-specific orientation and project context, see [CLAUDE.md](./CLAUDE.md).
> For what needs to be built next, see [spec/roadmap.md](./spec/roadmap.md).

## Project Shape

```
crates/coco-gateway/   — Gateway server: proxy, auth, token registry, admin API
crates/coco-cli/       — CLI: token management, shell activation, tool adapters
profiles/routes/       — Built-in route definitions (embedded at build time)
profiles/tools/        — Tool-specific activation adapters
docs/                  — Operational docs for the current state of the project
spec/                  — Vision, roadmap, and implementation specs
```

Current shipped route profiles are `openai`, `anthropic`, and `github`. TDX attestation, sealed credential storage, audit log, and `coco verify` are roadmap work, not current behavior.

## Development Rules

- Keep route and tool behavior consistent across `profiles/routes`, `profiles/tools`, CLI adapter output, README route table, and e2e tests.
- Prefer small, reversible diffs. Do not add dependencies unless explicitly needed.
- Do not duplicate long usage guides in README. README covers vision, current state, quickstart, and route table; detailed setup goes in `docs/USING.md`.

## Verification

- **Always:** `cargo fmt --check` and `cargo test --workspace`
- **When touching** gateway routing, auth, registry, Docker config, CLI activation, or README test instructions: `./scripts/test-e2e.sh`
- Live upstream checks (OpenAI, Anthropic, GitHub) are optional and skipped when credentials are absent.
