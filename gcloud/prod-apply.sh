#!/usr/bin/env bash
# Drift check and apply tool for the production Cloud Run service and the
# schema-sync job. See plan/prod-deploy.md for the design and rationale.
#
#   ./prod-apply.sh                      # --check for both targets (read-only)
#   ./prod-apply.sh --check [--web|--db|--all]
#   ./prod-apply.sh --apply --web        # replace the service manifest, deploy
#   ./prod-apply.sh --apply --db         # replace the job manifest, run the sync
#   ./prod-apply.sh --lint               # offline manifest checks, no gcloud
#
# `--apply` takes exactly one target; there is deliberately no combined apply, so
# application code can never ship in the same invocation as a schema update.
#
# Exit codes: 0 clean/success, 1 drift, 2 precondition or usage error, 3 apply or
# verification failure.
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(git -C "$SCRIPT_DIR" rev-parse --show-toplevel)"

# shellcheck source=prod-config.sh
source "$SCRIPT_DIR/prod-config.sh"
EXPECTED_SA=""

WEB_MANIFEST="$SCRIPT_DIR/prod-dreamscroll-web.yaml"
DB_MANIFEST="$SCRIPT_DIR/prod-dreamscroll-db.yaml"
CANONICAL_FILTER="$SCRIPT_DIR/prod-canonical.jq"

# Load-bearing variables; a manifest that lost any of these is a mistake, not a
# configuration change. Add to these lists deliberately.
REQUIRED_WEB_ENV=(
  NO_LOCAL_CONFIG_FILES RUST_LOG SERVICES
  GCLOUD_PROJECT_ID GCLOUD_REGION
  POSTGRES_USER POSTGRES_DB POSTGRES_HOST_PORT POSTGRES_CONNECTION_PARAMS POSTGRES_PASSWORD
  JWT_SECRET XAI_API_KEY
  STORAGE_BACKEND STORAGE_GCLOUD_BUCKET_NAME
  GEMINI_BACKEND GEMINI_MODEL_ID FIRESTARTER
  TASK_BACKEND TASK_MAX_ATTEMPTS TASK_WEBHOOK_BASE_URL
  TASK_QUEUE_NAME_ILLUMINATE TASK_QUEUE_NAME_SPARK TASK_QUEUE_NAME_SEARCH_INDEX
  TASK_OIDC_AUDIENCE TASK_OIDC_SERVICE_ACCOUNT_EMAIL
  SEARCH_EMBED_COLLECTION_ID SEARCH_EMBED_VECTOR_FIELD SEARCH_EMBED_VECTOR_DIMS
  SESSION_ALWAYS_SAVE COOKIE_SECURE
)
REQUIRED_DB_ENV=(
  NO_LOCAL_CONFIG_FILES RUST_LOG
  POSTGRES_HOST_PORT POSTGRES_USER POSTGRES_DB POSTGRES_CONNECTION_PARAMS
  POSTGRES_PASSWORD
)

MODE=check
TARGETS=()
OPT_IMAGE=""
WORKDIR=""

usage() {
  cat <<EOF
Usage: prod-apply.sh [mode] [target] [options]

Modes (default: --check):
  --check              Compare live Cloud Run config with the checked-in
                       manifests. Read-only. Exit 1 when they differ.
  --apply              Apply one manifest and verify the result.
  --lint               Render and validate both manifests offline (no gcloud).

Targets (--apply requires exactly one; --check defaults to both):
  --web                Service $SERVICE
  --db                 Job $JOB; also executes the schema sync
  --all                Both targets, --check only

Options:
  --image REF          Apply an explicit image tag or @sha256 digest instead of
                       the current commit. The rollback lever.
  -h, --help           This text.

Exit codes: 0 clean/success, 1 drift, 2 precondition/usage, 3 apply/verify failure.
EOF
}

log() { printf '==> %s\n' "$*" >&2; }
die() { printf 'error: %s\n' "$*" >&2; exit 2; }
fail() { printf 'error: %s\n' "$*" >&2; exit 3; }
invariant_violation() {
  if [[ "$MODE" == lint ]]; then die "$*"; else fail "$*"; fi
}

cleanup() {
  if [[ -n "$WORKDIR" ]]; then rm -rf "$WORKDIR"; fi
}
trap cleanup EXIT

make_workdir() {
  if [[ -z "$WORKDIR" ]]; then WORKDIR="$(mktemp -d)"; fi
  printf '%s' "$WORKDIR"
}

require_tools() {
  local need_gcloud="$1" tool ver
  for tool in jq yq; do
    command -v "$tool" >/dev/null 2>&1 || die "missing required tool: $tool"
  done
  if [[ "$need_gcloud" == yes ]]; then
    command -v gcloud >/dev/null 2>&1 || die "missing required tool: gcloud"
  fi
  ver="$(yq --version 2>/dev/null || true)"
  if [[ ! "$ver" =~ version\ +v?4\.([0-9]+) ]]; then
    die "yq v4 required (found: ${ver:-nothing})"
  fi
  (( BASH_REMATCH[1] >= 18 )) || die "yq >= 4.18 required for strenv() (found: $ver)"
  [[ -r "$CANONICAL_FILTER" ]] || die "missing $CANONICAL_FILTER"
  [[ -r "$WEB_MANIFEST" ]] || die "missing $WEB_MANIFEST"
  [[ -r "$DB_MANIFEST" ]] || die "missing $DB_MANIFEST"
}

# Committed manifests must not pin an image; prod-apply.sh injects the image of
# the commit being released, so a pinned image would silently defeat that. This
# is a repository error in every mode, hence `die` rather than an apply failure.
validate_source_manifests() {
  local file path status
  for file in "$WEB_MANIFEST" "$DB_MANIFEST"; do
    for path in '.spec.template.spec.containers[0]' '.spec.template.spec.template.spec.containers[0]'; do
      status=0
      yq -e "$path | has(\"image\")" "$file" >/dev/null 2>&1 || status=$?
      case "$status" in
        0) die "$(basename "$file") must not pin an image (prod-apply.sh injects it)" ;;
        1) ;;
        *) die "cannot evaluate $path in $(basename "$file") (yq exit $status)" ;;
      esac
    done
    for path in '.spec.template.spec' '.spec.template.spec.template.spec'; do
      status=0
      yq -e "$path | has(\"serviceAccountName\")" "$file" >/dev/null 2>&1 || status=$?
      case "$status" in
        0) die "$(basename "$file") must not pin a service account (Cloud Run defaults to the project's default)" ;;
        1) ;;
        *) die "cannot evaluate $path in $(basename "$file") (yq exit $status)" ;;
      esac
    done
  done
}

# Cloud Run defaults the service identity to the project's Compute Engine default
# service account. Derive it and verify the live resources use it, instead of
# committing the account address.
require_identity() {
  EXPECTED_SA="$(gcloud compute project-info describe \
    --format='value(defaultServiceAccount)' 2>/dev/null || true)"
  [[ -n "$EXPECTED_SA" ]] ||
    die "cannot determine the project's default service account"
}

assert_service_identity() {
  local live_sa
  live_sa="$(gcloud run services describe "$SERVICE" --region "$REGION" \
    --format='value(spec.template.spec.serviceAccountName)' 2>/dev/null || true)"
  if [[ "$live_sa" != "$EXPECTED_SA" ]]; then
    echo "drift: $SERVICE runs as '${live_sa:-<none>}', expected the project default '$EXPECTED_SA'" >&2
    return 1
  fi
  log "$SERVICE runs as $EXPECTED_SA"
}

assert_job_identity() {
  local live_sa
  live_sa="$(gcloud run jobs describe "$JOB" --region "$REGION" \
    --format='value(spec.template.spec.template.spec.serviceAccountName)' 2>/dev/null || true)"
  if [[ "$live_sa" != "$EXPECTED_SA" ]]; then
    echo "drift: $JOB runs as '${live_sa:-<none>}', expected the project default '$EXPECTED_SA'" >&2
    return 1
  fi
  log "$JOB runs as $EXPECTED_SA"
}

require_project() {
  local project
  project="$(gcloud config get-value project 2>/dev/null || true)"
  [[ "$project" == "$PROJECT" ]] ||
    die "gcloud project is '${project:-unset}', expected '$PROJECT'"
}

require_clean_tree() {
  [[ -z "$(git -C "$REPO_ROOT" status --porcelain)" ]] ||
    die "worktree is dirty; the released image must correspond to a commit"
}

# Resolve the image to release and prove it exists in Artifact Registry.
resolve_image() {
  local ref digest
  if [[ -n "$OPT_IMAGE" ]]; then
    ref="$OPT_IMAGE"
  else
    ref="$IMAGE_BASE:$(git -C "$REPO_ROOT" rev-parse HEAD)"
  fi
  case "$ref" in
    "$IMAGE_BASE":*|"$IMAGE_BASE"@*) ;;
    */*) die "image must live under $IMAGE_BASE (got: $ref)" ;;
    *) ref="$IMAGE_BASE:$ref" ;;
  esac
  digest="$(gcloud artifacts docker images describe "$ref" \
    --format='value(image_summary.digest)' 2>/dev/null || true)"
  [[ -n "$digest" ]] ||
    die "image not found in Artifact Registry: $ref (build and push it first)"
  log "image: $ref"
  log "digest: $digest"
  printf '%s' "$ref"
}

render_web() {
  IMAGE_REF="$1" yq -o=yaml \
    '.spec.template.spec.containers[0].image = strenv(IMAGE_REF)' \
    "$WEB_MANIFEST" >"$2"
}

render_db() {
  IMAGE_REF="$1" yq -o=yaml \
    '.spec.template.spec.template.spec.containers[0].image = strenv(IMAGE_REF)' \
    "$DB_MANIFEST" >"$2"
}

manifest_projection() {
  yq -p=yaml -o=json '.' "$1" | jq -S -rf "$CANONICAL_FILTER" | sort
}

live_service_projection() {
  gcloud run services describe "$SERVICE" --region "$REGION" --format=json |
    jq -S -rf "$CANONICAL_FILTER" | sort
}

check_service() {
  local live manifest ok=0
  live="$(live_service_projection)"
  manifest="$(manifest_projection "$WEB_MANIFEST")"
  if [[ "$live" == "$manifest" ]]; then
    log "$SERVICE matches $WEB_MANIFEST"
  else
    {
      echo "drift: live $SERVICE differs from $(basename "$WEB_MANIFEST")"
      diff -u --label manifest --label live \
        <(printf '%s\n' "$manifest") <(printf '%s\n' "$live") || true
    } >&2
    ok=1
  fi
  assert_service_identity || ok=1
  return "$ok"
}

check_db() {
  local live manifest out ok=0
  if ! out="$(gcloud run jobs describe "$JOB" --region "$REGION" --format=json 2>&1)"; then
    if grep -qiE 'not[ _-]?found|cannot find|does not exist|could not be found' <<<"$out"; then
      log "$JOB does not exist yet; run: $0 --apply --db"
      return 2
    fi
    die "jobs describe failed: $out"
  fi
  live="$(printf '%s' "$out" | jq -S -rf "$CANONICAL_FILTER" | sort)"
  manifest="$(manifest_projection "$DB_MANIFEST")"
  if [[ "$live" == "$manifest" ]]; then
    log "$JOB matches $DB_MANIFEST"
  else
    {
      echo "drift: live $JOB differs from $(basename "$DB_MANIFEST")"
      diff -u --label manifest --label live \
        <(printf '%s\n' "$manifest") <(printf '%s\n' "$live") || true
    } >&2
    ok=1
  fi
  assert_job_identity || ok=1
  return "$ok"
}

# yq exits 0 for true, 1 for false, >1 for an evaluation error. Distinguish the
# last case: a broken invariant expression must not masquerade as a manifest bug.
check_expr() {
  local file="$1" expr="$2" what="$3" status=0
  yq -e "$expr" "$file" >/dev/null 2>&1 || status=$?
  case "$status" in
    0) ;;
    1) invariant_violation "manifest invariant failed: $what" ;;
    *) invariant_violation "manifest invariant errored: $what (yq exit $status)" ;;
  esac
}

assert_web_invariants() {
  local file="$1" name
  check_expr "$file" "(.kind == \"Service\") and (.metadata.name == \"$SERVICE\")" \
    "service kind/name"
  check_expr "$file" '.spec.template.spec.containers[0] | has("image")' \
    "rendered image present"
  check_expr "$file" '.spec.template.spec.containers[0].image | length > 0' \
    "rendered image is not empty"
  check_expr "$file" \
    ".spec.template.metadata.annotations[\"run.googleapis.com/cloudsql-instances\"] == \"$SQL_INSTANCE\"" \
    "Cloud SQL attachment"
  check_expr "$file" '.metadata.annotations["run.googleapis.com/invoker-iam-disabled"] == "true"' \
    "public access annotation"
  check_expr "$file" '.spec.template.spec.containers[0].ports[0].containerPort == 8080' \
    "container port 8080"
  check_expr "$file" '.spec.template.spec.containers[0].startupProbe.tcpSocket.port == 8080' \
    "startup probe port 8080"
  check_expr "$file" \
    '.spec.traffic | ((length == 1) and (.[0].latestRevision == true) and (.[0].percent == 100))' \
    "traffic: 100% to latest"
  check_expr "$file" '.spec.template.spec.containers[0].env | length >= 25' \
    "env var count (truncated manifest?)"
  for name in "${REQUIRED_WEB_ENV[@]}"; do
    check_expr "$file" \
      ".spec.template.spec.containers[0].env | map(select(.name == \"$name\")) | length == 1" \
      "env $name present"
  done
  check_expr "$file" \
    '[.spec.template.spec.containers[0].env[] | select((.name | test("SECRET|PASSWORD|API_KEY|JWT")) and has("value"))] | length == 0' \
    "no literal values for secret-named vars"
  check_expr "$file" \
    '([.spec.template.spec.containers[0].env[] | select(.name == "POSTGRES_PASSWORD")][0]) as $e | (($e.valueFrom.secretKeyRef.name == "DREAMSCROLL_PROD_POSTGRES_PASSWORD") and ($e.valueFrom.secretKeyRef.key == "latest"))' \
    "POSTGRES_PASSWORD secret ref"
  check_expr "$file" \
    '([.spec.template.spec.containers[0].env[] | select(.name == "JWT_SECRET")][0]) as $e | (($e.valueFrom.secretKeyRef.name == "DREAMSCROLL_PROD_JWT_SECRET") and ($e.valueFrom.secretKeyRef.key == "latest"))' \
    "JWT_SECRET secret ref"
  check_expr "$file" \
    '([.spec.template.spec.containers[0].env[] | select(.name == "XAI_API_KEY")][0]) as $e | (($e.valueFrom.secretKeyRef.name == "DREAMSCROLL_PROD_XAI_API_KEY") and ($e.valueFrom.secretKeyRef.key == "latest"))' \
    "XAI_API_KEY secret ref"
}

assert_db_invariants() {
  local file="$1" name
  check_expr "$file" "(.kind == \"Job\") and (.metadata.name == \"$JOB\")" \
    "job kind/name"
  check_expr "$file" '.spec.template.spec.template.spec.containers[0] | has("image")' \
    "rendered image present"
  check_expr "$file" '.spec.template.spec.template.spec.containers[0].image | length > 0' \
    "rendered image is not empty"
  check_expr "$file" \
    '.spec.template.spec.template.spec.containers[0].command | length == 1' \
    "job command has one element"
  check_expr "$file" \
    '.spec.template.spec.template.spec.containers[0].command[0] == "/app/dreamscroll_db"' \
    "job command"
  check_expr "$file" \
    '.spec.template.spec.template.spec.containers[0].args | length == 1' \
    "job args has one element"
  check_expr "$file" \
    '.spec.template.spec.template.spec.containers[0].args[0] == "sync_schema"' \
    "job args"
  check_expr "$file" \
    ".spec.template.metadata.annotations[\"run.googleapis.com/cloudsql-instances\"] == \"$SQL_INSTANCE\"" \
    "Cloud SQL attachment"
  check_expr "$file" '.spec.template.spec.template.spec.maxRetries == 0' "maxRetries 0"
  check_expr "$file" '.spec.template.spec.template.spec.timeoutSeconds == 900' \
    "task timeout 900s"
  check_expr "$file" '.spec.template.spec.taskCount == 1' "taskCount 1"
  check_expr "$file" '.spec.template.spec.parallelism == 1' "parallelism 1"
  for name in "${REQUIRED_DB_ENV[@]}"; do
    check_expr "$file" \
      ".spec.template.spec.template.spec.containers[0].env | map(select(.name == \"$name\")) | length == 1" \
      "env $name present"
  done
  check_expr "$file" \
    '[.spec.template.spec.template.spec.containers[0].env[] | select((.name | test("SECRET|PASSWORD|API_KEY|JWT")) and has("value"))] | length == 0' \
    "no literal values for secret-named vars"
  check_expr "$file" \
    '([.spec.template.spec.template.spec.containers[0].env[] | select(.name == "POSTGRES_PASSWORD")][0]) as $e | (($e.valueFrom.secretKeyRef.name == "DREAMSCROLL_PROD_POSTGRES_PASSWORD") and ($e.valueFrom.secretKeyRef.key == "latest"))' \
    "POSTGRES_PASSWORD secret ref"
}

wait_service_ready() {
  local rev deadline json ready status percent
  rev="$(gcloud run services describe "$SERVICE" --region "$REGION" \
    --format='value(status.latestCreatedRevisionName)')"
  [[ -n "$rev" ]] || fail "cannot determine the latest created revision"
  deadline=$((SECONDS + 300))
  while :; do
    json="$(gcloud run services describe "$SERVICE" --region "$REGION" --format=json)"
    ready="$(printf '%s' "$json" | jq -r '.status.latestReadyRevisionName // ""')"
    percent="$(printf '%s' "$json" | jq -r '[.status.traffic[]?.percent] | add // 0')"
    status="$(gcloud run revisions describe "$rev" --region "$REGION" --format=json 2>/dev/null |
      jq -r '[.status.conditions[]? | select(.type == "Ready") | .status] | first // "Unknown"')"
    if [[ "$status" == "True" && "$ready" == "$rev" && "$percent" == "100" ]]; then
      log "revision $rev is ready and serving 100% of traffic"
      return 0
    fi
    ((SECONDS < deadline)) ||
      fail "revision $rev not ready after 300s (status=$status traffic=$percent%)"
    sleep 5
  done
}

apply_web() {
  local image dir
  require_clean_tree
  image="$(resolve_image)"
  dir="$(make_workdir)"
  render_web "$image" "$dir/web.yaml"
  assert_web_invariants "$dir/web.yaml"
  log "validating the manifest against the API (dry run)"
  gcloud run services replace "$dir/web.yaml" --region "$REGION" --dry-run >/dev/null
  log "applying $SERVICE"
  gcloud run services replace "$dir/web.yaml" --region "$REGION"
  wait_service_ready
  check_service || fail "post-apply drift for $SERVICE"
  log "service release complete"
}

apply_db() {
  local image dir rc=0 exec_name
  require_clean_tree
  image="$(resolve_image)"
  dir="$(make_workdir)"
  render_db "$image" "$dir/db.yaml"
  assert_db_invariants "$dir/db.yaml"
  log "applying $JOB"
  gcloud run jobs replace "$dir/db.yaml" --region "$REGION" >/dev/null
  log "executing $JOB (schema sync)"
  gcloud run jobs execute "$JOB" --region "$REGION" --wait || rc=$?
  exec_name="$(gcloud run jobs executions list --job "$JOB" --region "$REGION" \
    --limit 1 --format='value(name)' 2>/dev/null | awk -F/ 'END {print $NF}' || true)"
  if [[ -n "$exec_name" ]]; then
    log "execution: $exec_name"
    log "logs: gcloud beta run jobs executions logs read $exec_name --region $REGION"
  fi
  ((rc == 0)) || fail "schema sync execution failed (exit $rc)"
  check_db || fail "post-apply drift for $JOB"
  log "schema sync complete; 'prod-apply.sh --apply --web' is now unblocked"
}

lint_manifests() {
  local dir
  dir="$(make_workdir)"
  render_web "$IMAGE_BASE:lint" "$dir/web.yaml"
  render_db "$IMAGE_BASE:lint" "$dir/db.yaml"
  assert_web_invariants "$dir/web.yaml"
  assert_db_invariants "$dir/db.yaml"
  log "manifest lint passed (no cloud calls made)"
}

parse_args() {
  while (($#)); do
    case "$1" in
      --check) MODE=check ;;
      --apply) MODE=apply ;;
      --lint) MODE=lint ;;
      --web | --db) TARGETS+=("$1") ;;
      --all)
        [[ "$MODE" == check ]] || die "--apply takes exactly one target (--web or --db)"
        TARGETS+=(--web --db)
        ;;
      --image)
        OPT_IMAGE="${2:-}"
        [[ -n "$OPT_IMAGE" ]] || die "--image requires a value"
        shift
        ;;
      -h | --help) usage; exit 0 ;;
      *) usage >&2; die "unknown argument: $1" ;;
    esac
    shift
  done

  if [[ "$MODE" == lint ]]; then
    [[ -z "$OPT_IMAGE" ]] || die "--image is only valid with --apply"
    return 0
  fi

  [[ -z "$OPT_IMAGE" || "$MODE" == apply ]] || die "--image is only valid with --apply"

  if ((${#TARGETS[@]} == 0)); then
    [[ "$MODE" == check ]] || die "--apply requires --web or --db"
    TARGETS=(--web --db)
  fi
  if [[ "$MODE" == apply ]]; then
    ((${#TARGETS[@]} == 1)) || die "--apply takes exactly one target (--web or --db)"
  fi
}

main() {
  parse_args "$@"

  if [[ "$MODE" == lint ]]; then
    require_tools no
    validate_source_manifests
    lint_manifests
    exit 0
  fi

  require_tools yes
  require_project
  require_identity
  validate_source_manifests

  if [[ "$MODE" == check ]]; then
    local rc=0 code target
    for target in "${TARGETS[@]}"; do
      code=0
      case "$target" in
        --web) check_service || code=$? ;;
        --db) check_db || code=$? ;;
      esac
      case "$code" in
        0) ;;
        1) rc=1 ;;
        2) [[ "$rc" -eq 1 ]] || rc=2 ;;
        *) rc=$code ;;
      esac
    done
    if ((rc == 0)); then log "no drift"; fi
    exit "$rc"
  fi

  case "${TARGETS[0]}" in
    --web) apply_web ;;
    --db) apply_db ;;
    *) die "unknown target: ${TARGETS[0]}" ;;
  esac
}

main "$@"
