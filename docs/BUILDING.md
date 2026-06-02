# Building

## StageX OCI Images

Each shipped Rust binary has its own single-target Containerfile:

- `Containerfile.stagex` → `/usr/bin/enclave-credential-gateway` (the server; the
  only artifact deployed to the enclave)
- `Containerfile.cli.stagex` → `/usr/bin/gate` (client-side CLI, never deployed)

They are kept separate because Caution deploys the final stage of a Containerfile
and has no build-target directive — pointing it at a file whose sole output is the
server removes any ambiguity about what gets measured. Building the server alone
(`-p enclave-credential-gateway`) also avoids resolver-v2 cross-crate feature
unification with the CLI, so the enclave binary carries nothing CLI-driven.

Both build stages use the pinned linux/amd64 [StageX](https://codeberg.org/stagex/stagex)
Rust pallet digest documented in the Containerfiles. The release build runs after
`cargo fetch` with `--network=none`, `--frozen`, and `--release`. Runtime images
are `scratch` images containing only the statically linked binary.

```bash
./scripts/build-stagex-oci.sh
```

The script runs with `--no-cache` by default so every invocation produces a
clean, reproducible artifact. Set `ALLOW_CACHE=1` to reuse cached layers during
local iteration (faster, but may emit a stale hash that a clean reproduction
won't match):

```bash
ALLOW_CACHE=1 ./scripts/build-stagex-oci.sh
```

By default, this writes:

```text
dist/coco-credential-gateway-server.oci.tar
dist/coco-credential-gateway-cli.oci.tar
```

After every build the script prints the hashes, the exact command used to
produce them, and writes a per-commit sums file to the tracked `checksums/`
directory:

```text
checksums/sha256sums-<gitrev>.txt
```

Commit that file alongside the source change so the expected hashes travel
with the code.

### Reproducing and verifying a specific commit

Artifacts are pinned to the build commit: the script sets `SOURCE_DATE_EPOCH`
to that commit's timestamp, so every commit produces a distinct but
deterministic artifact. To reproduce, check out the **exact commit** named in
the checksum file and rebuild — the default epoch then matches automatically:

```bash
git checkout <gitrev>
./scripts/build-stagex-oci.sh --check
```

`--check` rebuilds and compares against the committed
`checksums/sha256sums-<gitrev>.txt`, exiting non-zero on any mismatch. You can
also verify an existing `dist/` build directly:

```bash
(cd dist && shasum -a 256 -c ../checksums/sha256sums-<gitrev>.txt)
```

Reproduce from the source commit, not the repository tip: a follow-up commit
that only records checksums still gets a new timestamp, and therefore a
different artifact. Tag the commits you publish (`git tag`) so released
artifacts have a stable pointer to reproduce from.

The OCI manifest digests inside the tarballs can be extracted with:

```bash
tar -xOf dist/coco-credential-gateway-server.oci.tar manifest.json | python3 -m json.tool
```

The script defaults `SOURCE_DATE_EPOCH` to the latest git commit timestamp
(`git log -1 --pretty=%ct`) so any two builds from the same commit produce
identical artifacts. Override it explicitly if needed:

```bash
SOURCE_DATE_EPOCH=0 TARGET_PLATFORM=linux/amd64 OUTPUT_DIR=dist IMAGE_PREFIX=coco-credential-gateway \
  ./scripts/build-stagex-oci.sh
```

To build one image manually (pass the git commit epoch explicitly so the value
is unambiguous):

```bash
SOURCE_DATE_EPOCH=$(git log -1 --pretty=%ct) docker buildx build \
  --platform linux/amd64 \
  --output type=oci,dest=dist/coco-credential-gateway-server.oci.tar,rewrite-timestamp=true \
  -f Containerfile.stagex .
```

Use `-f Containerfile.cli.stagex` and a different `dest` for the CLI image.

To check reproducibility against an existing `dist/` build, rebuild into a
separate directory and compare bytes:

```bash
OUTPUT_DIR=/tmp/coco-stagex-repro ./scripts/build-stagex-oci.sh
cmp -s dist/coco-credential-gateway-server.oci.tar /tmp/coco-stagex-repro/coco-credential-gateway-server.oci.tar
cmp -s dist/coco-credential-gateway-cli.oci.tar /tmp/coco-stagex-repro/coco-credential-gateway-cli.oci.tar
```

Then compare the tarballs with `cmp` or `shasum -a 256`.

linux/amd64 is the only supported reproducible target. The pinned StageX Rust
pallet (`sha256:2fbe7b…`) is a single-arch amd64 OCI image, not a multi-platform
index, so `--platform linux/arm64` has nothing to pull. On arm64 hosts the build
runs the amd64 pallet under emulation.

StageX's toolchain is arch-aware (`packages/core/rust/Containerfile` handles both
`amd64`/`x86_64` and `arm64`/`aarch64`), and both StageX Containerfiles keep the
matching `TARGETARCH` logic, but StageX does not publish a turnkey arm64 pallet
image. Producing arm64 artifacts would require building and pinning a StageX
arm64 rust pallet digest of your own — it is not a drop-in change.
