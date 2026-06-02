#!/bin/sh
set -eu

ROOT="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
IMAGE_PREFIX="${IMAGE_PREFIX:-coco-credential-gateway}"
OUTPUT_DIR="${OUTPUT_DIR:-${ROOT}/dist}"
SOURCE_DATE_EPOCH="${SOURCE_DATE_EPOCH:-$(git -C "$ROOT" log -1 --pretty=%ct 2>/dev/null || echo 0)}"
TARGET_PLATFORM="${TARGET_PLATFORM:-linux/amd64}"

MODE=record
case "${1:-}" in
	--check) MODE=check ;;
	-h|--help)
		echo "usage: $0 [--check]" >&2
		echo "  default: build artifacts and record checksums/sha256sums-<gitrev>.txt" >&2
		echo "  --check: rebuild and verify against the committed checksums for this commit" >&2
		exit 0
		;;
	"") ;;
	*) echo "unknown argument: $1 (try --help)" >&2; exit 2 ;;
esac

case "$OUTPUT_DIR" in
	/*) ;;
	*) OUTPUT_DIR="${ROOT}/${OUTPUT_DIR}" ;;
esac

mkdir -p "$OUTPUT_DIR"

# Build from scratch by default. A cached layer keeps the rewritten timestamps
# from the epoch it was first built at, so reusing it can emit a stale artifact
# hash that a clean third-party reproduction will not match. ALLOW_CACHE=1 trades
# that guarantee for speed during local iteration.
if [ "${ALLOW_CACHE:-}" = 1 ]; then
	CACHE_FLAG=
else
	CACHE_FLAG=--no-cache
fi

build_file() {
	file="$1"
	name="$2"

	SOURCE_DATE_EPOCH="$SOURCE_DATE_EPOCH" docker buildx build \
		$CACHE_FLAG \
		--platform "$TARGET_PLATFORM" \
		--output "type=oci,dest=${OUTPUT_DIR}/${name}.oci.tar,rewrite-timestamp=true" \
		-f "$ROOT/$file" \
		"$ROOT"
}

# Server is the only Caution enclave artifact; CLI is a separate client tool.
build_file Containerfile.stagex     "${IMAGE_PREFIX}-server"
build_file Containerfile.cli.stagex "${IMAGE_PREFIX}-cli"

GITREV="$(git -C "$ROOT" rev-parse --short HEAD 2>/dev/null || echo unknown)"
CHECKSUMS_DIR="${ROOT}/checksums"
SUMS_FILE="${CHECKSUMS_DIR}/sha256sums-${GITREV}.txt"
HASH_CMD="shasum -a 256"

printf '\nBuilt from commit %s (SOURCE_DATE_EPOCH=%s)\n' "$GITREV" "$SOURCE_DATE_EPOCH"

if [ "$MODE" = check ]; then
	if [ ! -f "$SUMS_FILE" ]; then
		printf 'No committed checksums for %s (%s).\n' "$GITREV" "$SUMS_FILE" >&2
		printf 'This commit was never certified; check out a certified commit or record it first.\n' >&2
		exit 1
	fi
	printf 'Verifying %s against %s\n\n' "$OUTPUT_DIR" "$SUMS_FILE"
	(cd "$OUTPUT_DIR" && $HASH_CMD -c "$SUMS_FILE")
	exit $?
fi

mkdir -p "$CHECKSUMS_DIR"

printf 'Hash command: (cd %s && %s %s-server.oci.tar %s-cli.oci.tar)\n\n' \
	"$OUTPUT_DIR" "$HASH_CMD" "$IMAGE_PREFIX" "$IMAGE_PREFIX"

(cd "$OUTPUT_DIR" && $HASH_CMD \
	"${IMAGE_PREFIX}-server.oci.tar" \
	"${IMAGE_PREFIX}-cli.oci.tar") | tee "$SUMS_FILE"

printf '\nWritten to: %s\n' "$SUMS_FILE"
printf 'Verify:     %s --check   (or: cd %s && %s -c %s)\n' "$0" "$OUTPUT_DIR" "$HASH_CMD" "$(basename "$SUMS_FILE")"
