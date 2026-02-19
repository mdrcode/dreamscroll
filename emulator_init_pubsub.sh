#!/usr/bin/env bash
set -euo pipefail

# Note that the push_endpoint differs based on the local topology:
# - When running localdev via cargo run, use host.docker.internal (to reach the host machine from the container)
# - When running cloudrun via Docker, use app (to reach the app container from the pubsub-emulator container)

PUBSUB_EMULATOR_BASE_URL="${PUBSUB_EMULATOR_BASE_URL:-http://localhost:8085}"
PUBSUB_PROJECT_ID="${PUBSUB_PROJECT_ID:-dreamscroll-local}"
PUBSUB_TOPIC_ID="${PUBSUB_TOPIC_ID:-dreamscroll-illumination}"
PUBSUB_SUBSCRIPTION_ID="${PUBSUB_SUBSCRIPTION_ID:-dreamscroll-illumination-push}"
#PUBSUB_PUSH_ENDPOINT="${PUBSUB_PUSH_ENDPOINT:-http://host.docker.internal:8080/webhook/illumination/push}"
PUBSUB_PUSH_ENDPOINT="${PUBSUB_PUSH_ENDPOINT:-http://app:8080/webhook/illumination/push}"

TOPIC_PATH="projects/${PUBSUB_PROJECT_ID}/topics/${PUBSUB_TOPIC_ID}"
SUB_PATH="projects/${PUBSUB_PROJECT_ID}/subscriptions/${PUBSUB_SUBSCRIPTION_ID}"

echo "Creating topic: ${TOPIC_PATH}"
curl -sS -X PUT "${PUBSUB_EMULATOR_BASE_URL}/v1/${TOPIC_PATH}" \
  -H 'Content-Type: application/json' \
  -d '{}' >/dev/null

echo "Creating push subscription: ${SUB_PATH} -> ${PUBSUB_PUSH_ENDPOINT}"
curl -sS -X PUT "${PUBSUB_EMULATOR_BASE_URL}/v1/${SUB_PATH}" \
  -H 'Content-Type: application/json' \
  -d "{\"topic\":\"${TOPIC_PATH}\",\"pushConfig\":{\"pushEndpoint\":\"${PUBSUB_PUSH_ENDPOINT}\"}}" >/dev/null

echo "Done."
echo "Topic: ${TOPIC_PATH}"
echo "Subscription: ${SUB_PATH}"
