#!/usr/bin/env bash

TAG=$(git rev-parse --short HEAD)
IMAGE_NAME="dreamscroll-web"

PROJECT=$(gcloud config get-value project 2>/dev/null)
if [[ -z "$PROJECT" || "$PROJECT" == "(unset)" ]]; then
	echo "Error: gcloud project is not set. Run: gcloud config set project <PROJECT_ID>" >&2
	exit 1
fi

LOCATION=$(gcloud config get-value artifacts/location 2>/dev/null)
if [[ -z "$LOCATION" || "$LOCATION" == "(unset)" ]]; then
	LOCATION="us-central1"
    echo "Using default artifacts location: $LOCATION"
fi

REPO=$(gcloud config get-value artifacts/repository 2>/dev/null)
if [[ -z "$REPO" || "$REPO" == "(unset)" ]]; then
	REPO="dreamscroll-repo"
    echo "Using default artifacts repository: $REPO"
fi

IMAGE_BASE="$LOCATION-docker.pkg.dev/$PROJECT/$REPO/$IMAGE_NAME"

docker build --platform linux/amd64 -t "$IMAGE_BASE:latest" -t "$IMAGE_BASE:$TAG" .

docker push "$IMAGE_BASE:latest"

docker push "$IMAGE_BASE:$TAG"