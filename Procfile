# Caution Procfile - https://docs.caution.co/reference/procfile/
# Enclave Credential Gateway (server) — reproducible StageX build.

# --- Build ---------------------------------------------------------------
# Caution runs `docker build -f <containerfile> .` and deploys the LAST stage.
# Containerfile.stagex must therefore END with the `server` stage — the `cli`
# stage is a client-side tool and must never be the deployed enclave image.
containerfile: Containerfile.stagex

# The server stage is a fully static (crt-static musl) binary on `scratch` with
# TLS roots compiled in (webpki-roots), so upstream HTTPS needs no CA bundle.
# Extract just the binary so PCR2 measures only the gateway, nothing else.
binary: /usr/bin/enclave-credential-gateway
# GATE_LISTEN_PORT=8083 because steve (e2e) forwards decrypted traffic to the
# hardcoded upstream 127.0.0.1:8083 — the gateway must listen there.
run: GATE_LISTEN_PORT=8083 /usr/bin/enclave-credential-gateway

# --- Source verification -------------------------------------------------
# Caution substitutes ${COMMIT}; lets `caution verify` reproduce from source.
app_sources: https://github.com/vkobel/enclave-credential-gateway/archive/${COMMIT}.tar.gz

# --- Networking ----------------------------------------------------------
# Gateway binds 0.0.0.0:8083 (GATE_LISTEN_PORT above). http_port puts
# Caution's Caddy in front with TLS on 443 — this replaces the repo's local
# Caddyfile / docker-compose caddy service in production.
ports: 8083
http_port: 8083

# --- Secrets -------------------------------------------------------------
# Real upstream credentials + admin token must be decrypted ONLY inside the
# enclave. Do NOT bake them into the image or this Procfile. Provision via
# Locksmith: GATE_ADMIN_TOKEN, OPENAI_API_KEY, ANTHROPIC_API_KEY, GITHUB_TOKEN.
locksmith: true

# --- Resources (defaults shown; fine for this proxy) ---------------------
# memory: 512
# cpus: 2

# --- Optional ------------------------------------------------------------
# domain: gateway.example.com
# steve handles /e2p/* on port 49500; the admin CLI uses it for encrypted admin ops.
# NOTE: no inline comments on value lines — the Procfile parser reads the full
# remainder of the line as the value (e2e would silently parse as false).
e2e: true
# debug: false   # NEVER true in prod: zeros PCRs and opens SSH on port 22.
