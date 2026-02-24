#!/usr/bin/env bash
set -euo pipefail

# Start the pubsub emulator using:
# `gcloud beta emulators pubsub start --host-port=localhost:8085``
#
# and then execute this script to create the topics/subscriptions

PROJECT_ID="${PUBSUB_PROJECT_ID:-dreamscroll_local}"

PUBSUB_EMULATOR_BASE_URL="${PUBSUB_EMULATOR_BASE_URL:-http://localhost:8085}"
PUBSUB_TOPIC_ID="${PUBSUB_TOPIC_ID:-dreamscroll-new_capture}"
PUBSUB_SUBSCRIPTION_ID="${PUBSUB_SUBSCRIPTION_ID:-dreamscroll-illumination-push}"
PUBSUB_PUSH_ENDPOINT="${PUBSUB_PUSH_ENDPOINT:-http://localhost:8080/webhook/illumination/push}"

TOPIC_PATH="projects/${PROJECT_ID}/topics/${PUBSUB_TOPIC_ID}"
SUB_PATH="projects/${PROJECT_ID}/subscriptions/${PUBSUB_SUBSCRIPTION_ID}"

echo "Creating topic: ${TOPIC_PATH}"
curl -sS -X PUT "${PUBSUB_EMULATOR_BASE_URL}/v1/${TOPIC_PATH}" \
  -H 'Content-Type: application/json' \
  -d '{}' >/dev/null

echo "Creating push subscription: ${SUB_PATH} -> ${PUBSUB_PUSH_ENDPOINT}"
curl -sS -X PUT "${PUBSUB_EMULATOR_BASE_URL}/v1/${SUB_PATH}" \
  -H 'Content-Type: application/json' \
  -d "{\"topic\":\"${TOPIC_PATH}\",\"ackDeadlineSeconds\":120,\"pushConfig\":{\"pushEndpoint\":\"${PUBSUB_PUSH_ENDPOINT}\"}}" >/dev/null

echo "Done."
echo "Topic: ${TOPIC_PATH}"
echo "Subscription: ${SUB_PATH}"
