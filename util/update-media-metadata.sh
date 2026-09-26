#!/usr/bin/env bash

set -euo pipefail

BUCKET="${STORAGE_GCLOUD_BUCKET_NAME:?Set STORAGE_GCLOUD_BUCKET_NAME to the target bucket}"
CACHE_CONTROL="private, max-age=604800, immutable"
APPLY=false

if [[ "${1:-}" == "--apply" ]]; then
    APPLY=true
    shift
fi

if [[ "$#" -gt 0 ]]; then
    echo "Usage: $0 [--apply]" >&2
    exit 2
fi

if [[ "$APPLY" != true ]]; then
    echo "Dry run. Re-run with --apply to update objects in gs://$BUCKET" >&2
fi

OBJECT_LIST=$(mktemp)
trap 'rm -f "$OBJECT_LIST"' EXIT

# Materialize the list before updating. This avoids hiding failures in a
# producer/consumer pipeline and makes the number of intended objects visible.
gcloud storage ls --recursive "gs://$BUCKET" \
    | grep -E '^gs://[^/]+/.+\.(gif|jpeg|jpg|png)$' > "$OBJECT_LIST"

total=$(wc -l < "$OBJECT_LIST" | tr -d ' ')
echo "Image objects found: $total"

updated=0
failed=0
while IFS= read -r object; do
    case "$object" in
        *.gif)  content_type="image/gif" ;;
        *.jpeg|*.jpg) content_type="image/jpeg" ;;
        *.png)  content_type="image/png" ;;
        *)      continue ;;
    esac

    if [[ "$APPLY" != true ]]; then
        printf '%s\t%s\t%s\n' "$object" "$content_type" "$CACHE_CONTROL"
        continue
    fi

    echo "Updating $object ($content_type)"
    if gcloud storage objects update "$object" \
        --cache-control="$CACHE_CONTROL" \
        --content-type="$content_type" >/dev/null \
        && metadata=$(gcloud storage objects describe "$object" --format=json) \
        && python3 -c 'import json, sys
d = json.load(sys.stdin)
expected_type = sys.argv[1]
expected_cache = sys.argv[2]
if d.get("content_type") != expected_type or d.get("cache_control") != expected_cache:
    raise SystemExit("metadata mismatch: content_type={!r} cache_control={!r}".format(d.get("content_type"), d.get("cache_control")))' \
            "$content_type" "$CACHE_CONTROL" <<< "$metadata"; then
        updated=$((updated + 1))
    else
        echo "FAILED verification: $object" >&2
        failed=$((failed + 1))
    fi
done < "$OBJECT_LIST"

if [[ "$APPLY" == true ]]; then
    echo "Updated: $updated; failed verification: $failed; found: $total"
    [[ "$failed" -eq 0 && "$updated" -eq "$total" ]]
fi
