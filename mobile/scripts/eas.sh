#!/usr/bin/env bash
# Runs eas as Rift's Expo account whatever `eas login` holds: the token in
# ~/.config/rift-release/expo-token (an access token of that account, mode 600)
# becomes EXPO_TOKEN for this command only. Without the file, eas runs as the
# logged-in user.
set -euo pipefail
token="${RIFT_EXPO_TOKEN_FILE:-$HOME/.config/rift-release/expo-token}"
if [[ -z "${EXPO_TOKEN:-}" && -r "$token" ]]; then
    EXPO_TOKEN="$(tr -d '[:space:]' <"$token")"
    export EXPO_TOKEN
fi
exec npx eas "$@"
