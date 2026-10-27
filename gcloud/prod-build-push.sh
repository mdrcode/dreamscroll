#!/usr/bin/env bash
# Build and push the production image for the current commit.
#
#   ./prod-build-push.sh            # build + push, print the release command
#   ./prod-build-push.sh --check    # preflight only, no build, no push
#
# The image tag is the commit SHA. The script prints the pushed digest so the
# release step can pin an immutable reference instead of re-resolving the tag.
# See plan/prod-deploy.md.
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=prod-config.sh
source "$SCRIPT_DIR/prod-config.sh"
REPO_ROOT="$(git -C "$SCRIPT_DIR" rev-parse --show-toplevel)"

MODE=build
TAG=""
REF=""

usage() {
  cat <<'EOF'
Usage: prod-build-push.sh [--check]

  (no args)   Build the image for linux/amd64, push it tagged with the current
              commit, verify the platform, and print the release command.
  --check     Run the preflight checks only: tools, project, clean worktree.
  -h, --help  This text.

Environment:
  ALLOW_DIRTY=1  Build anyway from a dirty worktree. The release step refuses
                 such an image, so this is only useful for inspection builds.
EOF
}

log() { printf '==> %s\n' "$*" >&2; }
die() { printf 'error: %s\n' "$*" >&2; exit 2; }
fail() { printf 'error: %s\n' "$*" >&2; exit 3; }

require_tools() {
  command -v docker >/dev/null 2>&1 || die "missing required tool: docker"
  command -v gcloud >/dev/null 2>&1 || die "missing required tool: gcloud"
  docker buildx version >/dev/null 2>&1 || die "docker buildx is required"
}

# The destination is declared in prod-config.sh, not inherited from whatever the
# local gcloud configuration happens to point at.
require_project() {
  local project
  project="$(gcloud config get-value project 2>/dev/null || true)"
  [[ "$project" == "$PROJECT" ]] ||
    die "gcloud project is '${project:-unset}', expected '$PROJECT'"
}

require_clean_tree() {
  [[ -z "$(git -C "$REPO_ROOT" status --porcelain)" ]] && return 0
  [[ "${ALLOW_DIRTY:-}" == "1" ]] && return 0
  die "worktree is dirty; the image would be tagged $TAG but contain other code (set ALLOW_DIRTY=1 to override)"
}

parse_args() {
  while (($#)); do
    case "$1" in
      --check) MODE=check ;;
      -h | --help) usage; exit 0 ;;
      *) usage >&2; die "unknown argument: $1" ;;
    esac
    shift
  done
}

# One step: build and push. Provenance is disabled so the pushed reference is a
# single image manifest whose digest is stable and directly deployable; enabling
# it later turns the reference into an index digest (which Cloud Run accepts).
build_push() {
  docker buildx build \
    --platform linux/amd64 \
    --push \
    --provenance=false \
    --file "$REPO_ROOT/Dockerfile" \
    --tag "$REF" \
    --label "org.opencontainers.image.revision=$TAG" \
    "$REPO_ROOT"
}

# A wrong-architecture push deploys fine and dies at runtime with "exec format
# error". A single-manifest push exposes the platform in the image config; an
# index (which provenance or multiple platforms would produce) lists it per
# manifest instead, so fall back to the rendered inspect output in that case.
assert_platform() {
  local actual rendered
  actual="$(docker buildx imagetools inspect "$REF" \
    --format '{{.Image.OS}}/{{.Image.Architecture}}' 2>/dev/null || true)"
  if [[ -z "$actual" || "$actual" == *"<no value>"* ]]; then
    rendered="$(docker buildx imagetools inspect "$REF" 2>/dev/null || true)"
    grep -q 'Platform: *linux/amd64' <<<"$rendered" ||
      fail "pushed image is not linux/amd64; inspect output: $rendered"
    return 0
  fi
  [[ "$actual" == "linux/amd64" ]] ||
    fail "pushed image platform is '$actual', expected linux/amd64"
}

resolve_digest() {
  local digest
  digest="$(gcloud artifacts docker images describe "$REF" \
    --format='value(image_summary.digest)' 2>/dev/null || true)"
  [[ -n "$digest" ]] || fail "cannot read the pushed digest for $REF"
  printf '%s' "$digest"
}

main() {
  parse_args "$@"
  require_tools
  require_project
  TAG="$(git -C "$REPO_ROOT" rev-parse HEAD)"
  REF="$IMAGE_BASE:$TAG"
  require_clean_tree

  if [[ "$MODE" == check ]]; then
    log "preflight ok"
    log "would build and push: $REF"
    log "context: $REPO_ROOT (Dockerfile, --platform linux/amd64)"
    exit 0
  fi

  log "building $REF for linux/amd64"
  build_push
  assert_platform

  local digest
  digest="$(resolve_digest)"
  log "pushed $REF"
  log "digest: $digest"
  cat >&2 <<EOF

Release with the immutable reference:
  $SCRIPT_DIR/prod-apply.sh --apply --db  --image $IMAGE_BASE@$digest
  $SCRIPT_DIR/prod-apply.sh --apply --web --image $IMAGE_BASE@$digest
EOF
}

main "$@"
