#!/bin/sh
set -eu

ROOT="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
IMAGE_PREFIX="${IMAGE_PREFIX:-coco-credential-gateway}"
OUTPUT_DIR="${OUTPUT_DIR:-${ROOT}/dist}"
SOURCE_DATE_EPOCH="${SOURCE_DATE_EPOCH:-$(git -C "$ROOT" log -1 --pretty=%ct 2>/dev/null || echo 0)}"
TARGET_PLATFORM="${TARGET_PLATFORM:-linux/amd64}"

case "$OUTPUT_DIR" in
	/*) ;;
	*) OUTPUT_DIR="${ROOT}/${OUTPUT_DIR}" ;;
esac

mkdir -p "$OUTPUT_DIR"

build_target() {
	target="$1"
	name="$2"

	SOURCE_DATE_EPOCH="$SOURCE_DATE_EPOCH" docker buildx build \
		--platform "$TARGET_PLATFORM" \
		--target "$target" \
		--output "type=oci,dest=${OUTPUT_DIR}/${name}.oci.tar,rewrite-timestamp=true" \
		-f "$ROOT/Containerfile.stagex" \
		"$ROOT"
}

build_target server "${IMAGE_PREFIX}-server"
build_target cli "${IMAGE_PREFIX}-cli"

GITREV="$(git -C "$ROOT" rev-parse --short HEAD 2>/dev/null || echo unknown)"
CHECKSUMS_DIR="${ROOT}/checksums"
SUMS_FILE="${CHECKSUMS_DIR}/sha256sums-${GITREV}.txt"
HASH_CMD="shasum -a 256"

mkdir -p "$CHECKSUMS_DIR"

printf '\nBuilt from commit %s (SOURCE_DATE_EPOCH=%s)\n' "$GITREV" "$SOURCE_DATE_EPOCH"
printf 'Hash command: (cd %s && %s %s-server.oci.tar %s-cli.oci.tar)\n\n' \
	"$OUTPUT_DIR" "$HASH_CMD" "$IMAGE_PREFIX" "$IMAGE_PREFIX"

(cd "$OUTPUT_DIR" && $HASH_CMD \
	"${IMAGE_PREFIX}-server.oci.tar" \
	"${IMAGE_PREFIX}-cli.oci.tar") | tee "$SUMS_FILE"

printf '\nWritten to: %s\n' "$SUMS_FILE"
printf 'Verify:     (cd %s && %s -c %s)\n' "$OUTPUT_DIR" "$HASH_CMD" "$(basename "$SUMS_FILE")"
