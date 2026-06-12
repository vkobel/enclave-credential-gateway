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

Only **`GATE_ADMIN_TOKEN`** needs Locksmith. Upstream API keys (`OPENAI_API_KEY`,
`ANTHROPIC_API_KEY`, `GITHUB_TOKEN`) are pushed at runtime via
`gate admin creds register` over the steve-encrypted admin channel — they live in
enclave RAM and never need to be provisioned at boot.

Each Locksmith secret becomes one `*.asc` file; the filename minus `.asc` is the
env var name.

## One-time: deploy keymaker

From the locksmith repo (separate deployment that mints quorum bundles):

```sh
caution init && git push caution main
export KEYMAKER_URL=https://<your-keymaker-deployment>
```

## Per-deployment

### 1. Build the keyring and mint the quorum bundle

Each operator exports their public key into a single `keyring.asc` file, then
`caution secret new` contacts the keymaker and writes `.caution/quorum-bundle.json`.

**Solo operator (threshold 1 of 1):**

```sh
gpg --export --armor you@example.com > keyring.asc
export KEYMAKER_URL=http://<your-keymaker-deployment>
caution secret new keyring.asc --threshold 1 --max 1
# → writes .caution/quorum-bundle.json
```

**Multi-operator quorum (e.g. 2 of 3):**

Each operator's key must have a **signing subkey**, an **encryption subkey**, and an
**authentication subkey**. The default `gpg --gen-key` produces only signing and
encryption — add an auth subkey in expert mode:

```sh
gpg --expert --edit-key alice@example.com
# gpg> addkey → (11) ECC (set your own capabilities) → toggle Sign OFF, Auth ON → Curve 25519 → save
```

If any key is missing one of the three capabilities, `caution secret new` will reject
the keyring with a "no Keymaker-eligible certificates" error.

Then build the keyring and mint the bundle:

```sh
gpg --export --armor alice@example.com  > keyring.asc
gpg --export --armor bob@example.com   >> keyring.asc
gpg --export --armor carol@example.com >> keyring.asc
caution secret new keyring.asc --threshold 2 --max 3
# → writes .caution/quorum-bundle.json
```

### 2. Encrypt secrets

Put plaintext values in `.env` (already gitignored):

```sh
# .env
GATE_ADMIN_TOKEN="my-super-secret-gate-admin-token"
```

Then encrypt all vars in `.env` to the bundle's recipient key in one command:

```sh
caution secret encrypt
# → writes .caution/secrets/GATE_ADMIN_TOKEN.asc (one file per var)
```

`.caution/quorum-bundle.json` and `.caution/secrets/*.asc` are safe to commit —
they are encrypted to the enclave-only key. Plaintext values never enter the repo.

```sh
git add .caution/
git commit -m "add quorum bundle and encrypted secrets"
```

### 3. Deploy and unlock

The `Procfile` is already correct — `locksmith: true` and `e2e: true` are set.
Caution injects the bundle and encrypted secrets into the rootfs; they are
decrypted at boot and exported into the `run:` command's environment.

```sh
git push caution main      # enclave boots, waits on TCP 49504
caution secret send-shard  # each operator taps their smartcard until threshold met
```

## Rules

- Never set `debug: true` with Locksmith. Debug zeros PCRs, so the attestation
  `locksmithd` presents to shard-holders no longer proves which image is running —
  operators would release shards to an unmeasured enclave. Drop `ssh_keys` too.
- The master secret lives only in enclave RAM. Every reboot, redeploy, or instance
  replacement requires shard-holders to `send-shard` again. Keep `threshold` ≤ the
  number of operators reliably on call.
- The `Containerfile.stagex` server stage bakes no secrets and needs no change.
