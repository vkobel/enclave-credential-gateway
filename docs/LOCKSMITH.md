# Locksmith Secret Provisioning (Production)

Production secrets are provisioned via Caution Locksmith, not baked into the image
or the Procfile. The master secret is Shamir-sharded across OpenPGP-smartcard holders
and reconstituted in memory inside the attested enclave at boot.

> Locksmith is the **quorum / multi-operator** provisioning path: releasing secrets
> requires a threshold of shard-holders, so no single operator can do it alone. For a
> **solo operator**, the roadmap's owner-direct attested injection over steve
> (`gate creds push`) is the lighter alternative — it gates release on a reproduced
> measurement match rather than a human quorum. See
> [spec/roadmap.md](../spec/roadmap.md) and [spec/tee-security.md](../spec/tee-security.md) (R6).

## Secrets the gateway needs

`GATE_ADMIN_TOKEN`, `OPENAI_API_KEY`, `ANTHROPIC_API_KEY`, `GITHUB_TOKEN`.

Each becomes one `*.asc` file; the filename minus `.asc` is the env var name.

## One-time: deploy keymaker

From the locksmith repo (separate deployment that mints quorum bundles):

```sh
caution init && git push caution main
export KEYMAKER_URL=https://<your-keymaker-deployment>
```

## Per-deployment

1. Build the quorum keyring from operator public keys and mint the bundle:

   ```sh
   gpg --export --armor alice@example.com  > keyring.asc
   gpg --export --armor bob@example.com   >> keyring.asc
   gpg --export --armor carol@example.com >> keyring.asc
   caution secret new keyring.asc --threshold 2 --max 3   # writes .caution/quorum-bundle.json
   ```

2. Encrypt secrets to the bundle recipient:

   ```sh
   jq -r '.secret_recipient_public_key' .caution/quorum-bundle.json > recipient.asc
   mkdir -p .caution/secrets
   for NAME in GATE_ADMIN_TOKEN OPENAI_API_KEY ANTHROPIC_API_KEY GITHUB_TOKEN; do
     printf '%s' "${!NAME}" | gpg --batch --yes --trust-model always \
       --encrypt --armor --recipient-file recipient.asc \
       --output ".caution/secrets/${NAME}.asc"
   done
   ```

   `.caution/quorum-bundle.json` and `.caution/secrets/*.asc` are safe to commit
   (encrypted to the enclave-only key). Plaintext values never enter the repo.

3. Procfile — Locksmith on, no baked secrets, no debug:

   ```procfile
   containerfile: Containerfile.stagex
   run: GATE_TOKENS_FILE=/tokens.json /usr/bin/enclave-credential-gateway
   app_sources: https://github.com/vkobel/enclave-credential-gateway/archive/${COMMIT}.tar.gz
   http_port: 8080
   ports: 8080
   locksmith: true
   ```

   `GATE_TOKENS_FILE` is config, not a secret, so it stays inline. Caution injects the
   locksmith binaries and the `.caution/` bundle/secrets into the rootfs; the four
   secret vars are decrypted at boot and exported into the run command's environment.

4. Deploy and unlock:

   ```sh
   git push caution main      # enclave boots, waits on TCP 49504
   caution secret send-shard  # each operator, with their smartcard, until threshold met
   ```

## Rules

- Never set `debug: true` with Locksmith. Debug zeros PCRs, so the attestation
  `locksmithd` presents to shard-holders no longer proves which image is running —
  operators would release shards to an unmeasured enclave. Drop `ssh_keys` too.
- The master secret lives only in enclave RAM. Every reboot, redeploy, or instance
  replacement requires shard-holders to `send-shard` again. Keep `threshold` ≤ the
  number of operators reliably on call.
- The `Containerfile.stagex` server stage bakes no secrets and needs no change.
