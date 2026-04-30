# Built-in Profiles

The gateway and CLI embed built-in profiles at build time. Add one file per
route or tool adapter:

```text
profiles/
  routes/<route>.yaml
  tools/<tool>.yaml
```

The filename stem is the route or tool key. For example,
`profiles/tools/aider.yaml` defines the `aider` adapter used by
`coco activate --tool aider`.

## Route Files

Route files contain the body of one route entry:

```yaml
upstream: https://api.example.com
credential_sources:
  - env: EXAMPLE_TOKEN
    inject_header: Authorization
    format: "Bearer {}"
```

Supported route fields are `upstream`, `credential_sources`, `aliases`,
`inject_mode`, `url_path_prefix`, and `git_protocol`.

## Tool Files

Tool files contain the body of one tool adapter entry:

```yaml
description: Aider AI pair programmer
routes: [anthropic, openai]
env:
  - requires_route: anthropic
    key: ANTHROPIC_API_KEY
    value: "{{token}}"
  - requires_route: anthropic
    key: ANTHROPIC_API_BASE
    value: "{{route_url:anthropic}}"
  - requires_route: openai
    key: OPENAI_API_KEY
    value: "{{token}}"
  - requires_route: openai
    key: OPENAI_API_BASE
    value: "{{route_url:openai}}/v1"
```

Supported tool fields are `description`, `routes`, `git_credential_helper`,
`env`, and `files`. Template variables currently supported by tool values are
`{{token}}`, `{{token_name}}`, `{{gateway_url}}`, `{{gateway_host}}`,
`{{generated_root}}`, `{{route_url:<route>}}`, and
`{{managed_file:<id>}}`.

Rebuild the binaries after adding, removing, or changing profile files.
