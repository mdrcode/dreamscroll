#!/usr/bin/env bash
set -euo pipefail

# Start the pubsub emulator using:
# `gcloud beta emulators pubsub start --host-port=localhost:8085`
#
# and then execute this script to create the topics/subscriptions

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "${SCRIPT_DIR}/ds_config_local.env"

: "${GCLOUD_PROJECT_ID:?GCLOUD_PROJECT_ID must be set in ds_config_local.env}"
: "${PUBSUB_EMULATOR:?PUBSUB_EMULATOR must be set in ds_config_local.env}"
: "${PUBSUB_TOPIC_ID_NEW_CAPTURE:?PUBSUB_TOPIC_ID_NEW_CAPTURE must be set in ds_config_local.env}"

PUBSUB_SUBSCRIPTION_ID="dreamscroll-illumination-push"
PUBSUB_PUSH_ENDPOINT="http://localhost:8080/_wh/illumination/push"

TOPIC_PATH="projects/${GCLOUD_PROJECT_ID}/topics/${PUBSUB_TOPIC_ID_NEW_CAPTURE}"
SUB_PATH="projects/${GCLOUD_PROJECT_ID}/subscriptions/${PUBSUB_SUBSCRIPTION_ID}"

echo "Creating topic: ${TOPIC_PATH}"
curl -sS -X PUT "${PUBSUB_EMULATOR}/v1/${TOPIC_PATH}" \
  -H 'Content-Type: application/json' \
  -d '{}' >/dev/null

echo "Creating push subscription: ${SUB_PATH} -> ${PUBSUB_PUSH_ENDPOINT}"
curl -sS -X PUT "${PUBSUB_EMULATOR}/v1/${SUB_PATH}" \
  -H 'Content-Type: application/json' \
  -d "{\"topic\":\"${TOPIC_PATH}\",\"ackDeadlineSeconds\":120,\"pushConfig\":{\"pushEndpoint\":\"${PUBSUB_PUSH_ENDPOINT}\"}}" >/dev/null

echo "Done."
echo "Topic: ${TOPIC_PATH}"
echo "Subscription: ${SUB_PATH}"
