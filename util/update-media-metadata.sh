#!/usr/bin/env bash

set -euo pipefail

BUCKET="${STORAGE_GCLOUD_BUCKET_NAME:-dreamscroll-prod-media1}"
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

gcloud storage ls --recursive "gs://$BUCKET" \
    | grep -E '^gs://[^/]+/.+\.(gif|jpeg|jpg|png)$' \
    | while IFS= read -r object; do
        case "$object" in
            *.gif)  content_type="image/gif" ;;
            *.jpeg) content_type="image/jpeg" ;;
            *.jpg)  content_type="image/jpeg" ;;
            *.png)  content_type="image/png" ;;
            *)      continue ;;
        esac

        if [[ "$APPLY" == true ]]; then
            echo "Updating $object ($content_type)"
            gcloud storage objects update "$object" \
                --cache-control="$CACHE_CONTROL" \
                --content-type="$content_type"
        else
            printf '%s\t%s\t%s\n' "$object" "$content_type" "$CACHE_CONTROL"
        fi
    done
