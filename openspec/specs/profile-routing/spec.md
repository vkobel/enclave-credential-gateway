## ADDED Requirements

### Requirement: Profile file defines the route table
The gateway SHALL load its route table from a JSON profile file at startup. The profile path SHALL be resolved in this order: (1) `COCO_PROFILE` env var, (2) `/etc/coco/profile.json`, (3) built-in defaults. The gateway SHALL log which source was used.

#### Scenario: Profile loaded from default path
- **WHEN** `/etc/coco/profile.json` exists and is valid JSON
- **THEN** the gateway loads routes from the file and logs "loaded N routes from profile"

#### Scenario: Profile loaded from env var override
- **WHEN** `COCO_PROFILE=/custom/path.json` is set and the file exists
- **THEN** the gateway loads routes from that path, ignoring `/etc/coco/profile.json`

#### Scenario: No profile file found — built-in fallback
- **WHEN** neither `COCO_PROFILE` nor `/etc/coco/profile.json` exists
- **THEN** the gateway loads the built-in default routes (openai, anthropic, github) and logs "no profile found, using built-in defaults"

#### Scenario: Malformed profile JSON — startup failure
- **WHEN** the profile file exists but contains invalid JSON or fails schema validation
- **THEN** the gateway logs the parse error and exits with a non-zero status before accepting any connections

### Requirement: Profile schema — routes map
The profile JSON SHALL conform to the following structure:

```json
{
  "routes": {
    "<prefix>": {
      "upstream": "<https-url>",
      "credential_env": "<ENV_VAR_NAME>",
      "inject_header": "<header-name>",
      "credential_format": "<format-string>"
    }
  }
}
```

`upstream` and `credential_env` are required per route. `inject_header` defaults to `Authorization`. `credential_format` defaults to `Bearer {}` where `{}` is replaced with the credential value.

#### Scenario: Minimal route definition
- **WHEN** a route entry has only `upstream` and `credential_env`
- **THEN** the gateway applies default injection (`Authorization: Bearer <value>`)

#### Scenario: Custom inject header
- **WHEN** a route entry sets `"inject_header": "x-api-key"` and `"credential_format": "{}"`
- **THEN** the forwarded request carries `x-api-key: <credential>` instead of `Authorization`

#### Scenario: Route with missing credential_env field
- **WHEN** a route entry in the profile omits `credential_env`
- **THEN** the gateway logs a warning and skips that route (does not fail startup)

### Requirement: Profile replaces built-ins entirely
When a profile file is loaded, the built-in routes (openai, anthropic, github) SHALL NOT be active unless explicitly defined in the profile.

#### Scenario: Profile-only routes active
- **WHEN** the profile defines only a `httpbin` route
- **THEN** requests to `/openai/` return 404 and requests to `/httpbin/` are proxied

#### Scenario: Operator replicates built-ins in profile
- **WHEN** the profile defines `openai`, `anthropic`, and `github` routes
- **THEN** those routes behave identically to the built-in defaults
