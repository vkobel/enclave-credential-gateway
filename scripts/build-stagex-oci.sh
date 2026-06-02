#!/bin/sh
set -eu

ROOT="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
IMAGE_PREFIX="${IMAGE_PREFIX:-coco-credential-gateway}"
OUTPUT_DIR="${OUTPUT_DIR:-${ROOT}/dist}"
SOURCE_DATE_EPOCH="${SOURCE_DATE_EPOCH:-$(git -C "$ROOT" log -1 --pretty=%ct 2>/dev/null || echo 0)}"
TARGET_PLATFORM="${TARGET_PLATFORM:-linux/amd64}"
HASH_CMD="shasum -a 256"

usage() {
	cat >&2 <<-EOF
	usage: $0 [--tag <name> | --check [tag]]

	  (default)        build both artifacts and print their hashes
	  --tag <name>     build, then record the hashes in an annotated git tag
	                   <name> on HEAD (the expected hashes live on the tag, not
	                   in the repo tree, so no follow-up commit is needed)
	  --check [tag]    rebuild and verify against the tag's recorded hashes;
	                   defaults to the artifact-bearing tag pointing at HEAD

	Artifacts are pinned to the build commit via SOURCE_DATE_EPOCH (its
	committer timestamp). Builds are --no-cache by default; ALLOW_CACHE=1 opts
	into caching for fast local iteration of the default build only (it is
	rejected with --tag and --check, which must reproduce from a clean build).
	EOF
}

MODE=build
TAG=
case "${1:-}" in
	--check) MODE=check; TAG="${2:-}" ;;
	--tag)
		MODE=tag
		TAG="${2:-}"
		[ -n "$TAG" ] || { echo "--tag requires a tag name" >&2; exit 2; }
		;;
	-h|--help) usage; exit 0 ;;
	"") ;;
	*) echo "unknown argument: $1 (try --help)" >&2; exit 2 ;;
esac

# Certify (--tag) and verify (--check) must reproduce from a clean build; a
# cached layer carries stale timestamps and would emit a hash a third party
# can't match. ALLOW_CACHE is only for fast local iteration of the default build.
if { [ "$MODE" = tag ] || [ "$MODE" = check ]; } && [ "${ALLOW_CACHE:-}" = 1 ]; then
	echo "refusing $MODE with ALLOW_CACHE=1: certify/verify must use a clean build" >&2
	exit 2
fi

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

# The lines a verifier feeds to `shasum -c`: "<hash>  <artifact>".
sums() {
	(cd "$OUTPUT_DIR" && $HASH_CMD \
		"${IMAGE_PREFIX}-server.oci.tar" \
		"${IMAGE_PREFIX}-cli.oci.tar")
}

printf '\nBuilt from commit %s (SOURCE_DATE_EPOCH=%s)\n\n' "$GITREV" "$SOURCE_DATE_EPOCH"

case "$MODE" in
build)
	sums
	printf '\nCertify:  %s --tag <name>\n' "$0"
	printf 'Verify:   %s --check\n' "$0"
	;;

tag)
	BODY="$(sums)"
	git -C "$ROOT" tag -a "$TAG" -m "stagex OCI artifacts ${GITREV}

SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH}
TARGET_PLATFORM=${TARGET_PLATFORM}

${BODY}"
	printf 'Tagged %s on %s with:\n\n%s\n\n' "$TAG" "$GITREV" "$BODY"
	printf 'Push the tag to publish:  git push origin %s\n' "$TAG"
	;;

check)
	if [ -z "$TAG" ]; then
		# Pick the tag at HEAD whose message carries our artifact hashes.
		for t in $(git -C "$ROOT" tag --points-at HEAD); do
			if git -C "$ROOT" cat-file tag "$t" 2>/dev/null | grep -q "${IMAGE_PREFIX}-server.oci.tar"; then
				TAG="$t"; break
			fi
		done
	fi
	if [ -z "$TAG" ]; then
		printf 'No artifact tag found on %s. Certify it first: %s --tag <name>\n' "$GITREV" "$0" >&2
		exit 1
	fi
	EXPECTED="$(git -C "$ROOT" cat-file tag "$TAG" | grep -E '\.oci\.tar$' || true)"
	if [ -z "$EXPECTED" ]; then
		printf 'Tag %s carries no artifact hashes.\n' "$TAG" >&2
		exit 1
	fi
	printf 'Verifying %s against tag %s\n\n' "$OUTPUT_DIR" "$TAG"
	printf '%s\n' "$EXPECTED" | (cd "$OUTPUT_DIR" && $HASH_CMD -c -)
	;;
esac
