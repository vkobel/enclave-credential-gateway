## Context

`coco-gateway` currently has three routes hardcoded in `main.rs`. Any new upstream requires editing Rust source and rebuilding the image. The profile system replaces this with a JSON file mounted into the container, read once at startup, with no code changes required for new routes.

The existing `RouteEntry` struct and `AppState` are the only code paths affected. The phantom-token middleware and proxy handler are unchanged — they already operate generically on whatever routes are in `AppState`.

## Goals / Non-Goals

**Goals:**
- Load route table from a JSON profile file at startup
- Support per-route `inject_header` and `credential_format` overrides
- Fall back to built-in defaults when no profile is found
- No new Cargo dependencies

**Non-Goals:**
- Hot-reload (profile is read once at boot)
- Keychain / secret manager integration (env vars only for now)
- Multiple profile files or merging
- Route-level TLS CA overrides (Phase 1b concern)

## Decisions

### D1: Profile replaces built-ins (not merges)

**Decision:** When a profile file is present, it is the complete route table. Built-ins are ignored.

**Rationale:** Merge semantics are ambiguous — if the profile defines an `openai` route and the built-in also exists, which wins? Replacement is predictable. Operators who want the defaults put them in their profile.

**Alternative:** Always include built-ins, profile adds/overrides. Rejected — hidden defaults cause confusion in production deployments.

### D2: Profile path via `COCO_PROFILE` env var, default `/etc/coco/profile.json`

**Decision:** Check `COCO_PROFILE` env var; fall back to `/etc/coco/profile.json`; fall back to built-ins if file not found.

**Rationale:** Standard Docker pattern — mount a config file at a known path. Env var override allows flexibility without rebuilding the image.

### D3: Minimal profile schema — flat route map, optional injection fields

**Decision:**
```json
{
  "routes": {
    "<prefix>": {
      "upstream": "https://...",
      "credential_env": "ENV_VAR_NAME",
      "inject_header": "Authorization",
      "credential_format": "Bearer {}"
    }
  }
}
```

`inject_header` defaults to `Authorization`. `credential_format` defaults to `Bearer {}`. `credential_env` is required; if absent the route is skipped with a warning.

**Rationale:** Matches the nono profile `custom_credentials` shape closely, adapted for env-var credentials. Keeps serde deserialization trivial — no custom visitors needed.

**Alternative:** Array of routes with explicit `prefix` field. Rejected — map keyed by prefix is more ergonomic and avoids duplicate-prefix bugs.

### D4: Missing credential env var at request time → 503, not startup failure

**Decision:** If `credential_env` is set but the env var is empty/absent at request time, return 503. Don't fail startup.

**Rationale:** Consistent with existing behavior. Operators can start the gateway and add credentials later. Startup failure only for missing `COCO_PHANTOM_TOKEN`.

## Risks / Trade-offs

- **Profile file not mounted** → falls back to built-ins silently. Mitigated by logging clearly which source (profile or built-ins) routes were loaded from.
- **Malformed JSON** → gateway refuses to start. This is intentional — bad config should be loud.
- **`{}` in `credential_format` is a simple string replace** → format strings with multiple `{}` or escaped braces are not supported. Documented limitation.
