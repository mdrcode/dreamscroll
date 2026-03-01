#!/usr/bin/env bash
set -euo pipefail

API_BASE_URL="${API_BASE_URL:-https://dreamscroll.ai}"
SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
TOKEN_ENV_FILE="${TOKEN_ENV_FILE:-$SCRIPT_DIR/src/rest/.env}"
PRINT_TOKEN="${PRINT_TOKEN:-0}"

usage() {
	echo "Usage: $0 [--username <username>]" >&2
}

cleanup() {
	unset DS_PASS DS_USER RESPONSE_BODY ACCESS_TOKEN
}
trap cleanup EXIT INT TERM

for command in curl jq; do
	if ! command -v "$command" >/dev/null 2>&1; then
		echo "Error: required command '$command' is not installed." >&2
		exit 1
	fi
done

DS_USER=""
while [[ $# -gt 0 ]]; do
	case "$1" in
		--username)
			if [[ $# -lt 2 || -z "${2:-}" ]]; then
				echo "Error: --username requires a value." >&2
				usage
				exit 1
			fi
			DS_USER="$2"
			shift 2
			;;
		-h|--help)
			usage
			exit 0
			;;
		*)
			echo "Error: unknown argument '$1'." >&2
			usage
			exit 1
			;;
	esac
done

if [[ -z "$DS_USER" ]]; then
	read -r -p "Username: " DS_USER
fi

read -r -s -p "Password: " DS_PASS
echo

RESPONSE_BODY="$({
	jq -nc --arg username "$DS_USER" --arg password "$DS_PASS" '{username:$username,password:$password}'
} | curl --silent --show-error --fail-with-body \
	--connect-timeout 10 --max-time 30 \
	"$API_BASE_URL/api/token" \
	-H 'Content-Type: application/json' \
	--data-binary @-)"

ACCESS_TOKEN="$(jq -r '.access_token // empty' <<<"$RESPONSE_BODY")"
if [[ -z "$ACCESS_TOKEN" ]]; then
	echo "Error: token not found in response." >&2
	jq -r '.error // .message // "Response body did not contain access_token"' <<<"$RESPONSE_BODY" >&2 || true
	exit 1
fi

umask 077
printf 'PROD_API_TOKEN=%s\n' "$ACCESS_TOKEN" > "$TOKEN_ENV_FILE"

printf 'Wrote token to %s\n' "$TOKEN_ENV_FILE" >&2
if [[ "$PRINT_TOKEN" == "1" ]]; then
	printf '%s\n' "$ACCESS_TOKEN"
fi