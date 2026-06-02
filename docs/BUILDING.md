# Building

## StageX OCI Images

`Containerfile.stagex` builds OCI image tarballs for both shipped Rust binaries:

- `server` target: `/usr/bin/enclave-credential-gateway`
- `cli` target: `/usr/bin/gate`

The build stage uses the pinned linux/amd64 [StageX](https://codeberg.org/stagex/stagex)
Rust pallet digest documented in the Containerfile. The release build runs after
`cargo fetch` with `--network=none`, `--frozen`, and `--release`. Runtime images
are `scratch` images containing only the statically linked binary.

```bash
./scripts/build-stagex-oci.sh
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

To build one target manually (pass the git commit epoch explicitly so the value
is unambiguous):

```bash
SOURCE_DATE_EPOCH=$(git log -1 --pretty=%ct) docker buildx build \
  --platform linux/amd64 \
  --target server \
  --output type=oci,dest=dist/coco-credential-gateway-server.oci.tar,rewrite-timestamp=true \
  -f Containerfile.stagex .
```

Use the `cli` target and a different `dest` for the CLI image.

To check reproducibility against an existing `dist/` build, rebuild into a
separate directory and compare bytes:

```bash
OUTPUT_DIR=/tmp/coco-stagex-repro ./scripts/build-stagex-oci.sh
cmp -s dist/coco-credential-gateway-server.oci.tar /tmp/coco-stagex-repro/coco-credential-gateway-server.oci.tar
cmp -s dist/coco-credential-gateway-cli.oci.tar /tmp/coco-stagex-repro/coco-credential-gateway-cli.oci.tar
```

For a stronger check, force one no-cache server build. The StageX build stage
compiles both binaries, so the CLI target can then be exported from that rebuilt
layer:

```bash
SOURCE_DATE_EPOCH=$(git log -1 --pretty=%ct) docker buildx build \
  --no-cache \
  --platform linux/amd64 \
  --target server \
  --output type=oci,dest=/tmp/coco-stagex-repro/coco-credential-gateway-server.oci.tar,rewrite-timestamp=true \
  -f Containerfile.stagex .

SOURCE_DATE_EPOCH=$(git log -1 --pretty=%ct) docker buildx build \
  --platform linux/amd64 \
  --target cli \
  --output type=oci,dest=/tmp/coco-stagex-repro/coco-credential-gateway-cli.oci.tar,rewrite-timestamp=true \
  -f Containerfile.stagex .
```

Then compare the tarballs with `cmp` or `shasum -a 256`.

linux/amd64 is the only supported reproducible target. The pinned StageX Rust
pallet (`sha256:2fbe7b…`) is a single-arch amd64 OCI image, not a multi-platform
index, so `--platform linux/arm64` has nothing to pull. On arm64 hosts the build
runs the amd64 pallet under emulation.

StageX's toolchain is arch-aware (`packages/core/rust/Containerfile` handles both
`amd64`/`x86_64` and `arm64`/`aarch64`), and our `Containerfile.stagex` keeps the
matching `TARGETARCH` logic, but StageX does not publish a turnkey arm64 pallet
image. Producing arm64 artifacts would require building and pinning a StageX
arm64 rust pallet digest of your own — it is not a drop-in change.
