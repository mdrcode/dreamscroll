# Shared production constants for gcloud/prod-apply.sh and
# gcloud/prod-build-push.sh. Sourced, never executed.
#
# Everything the two scripts must agree on lives here: when the build
# destination and the release destination drift apart, the only symptom is
# "image not found" at apply time.

PROJECT=mdrcode
REGION=us-central1
AR_LOCATION=us-central1
AR_REPO=dreamscroll-repo
IMAGE_NAME=dreamscroll-web
SERVICE=dreamscroll-webui-api
JOB=dreamscroll-db-sync
SQL_INSTANCE="$PROJECT:$REGION:dreamscroll-prod-db4"

IMAGE_BASE="$AR_LOCATION-docker.pkg.dev/$PROJECT/$AR_REPO/$IMAGE_NAME"
