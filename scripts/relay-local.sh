#!/usr/bin/env bash
# A temporary relay on this machine, for reaching it from a phone outside
# the local network until a hosted relay exists: the rift-relay container
# (packaging/relay.Dockerfile) behind a Cloudflare quick tunnel, which gives
# it a public https:// address without an account or a router change.
#
#   scripts/relay-local.sh             # build the image if missing, then run
#   scripts/relay-local.sh --rebuild   # rebuild the image first
#
# Needs docker and cloudflared (`brew install cloudflared`). While it runs,
# config.toml's `remote_relay` points at the tunnel's wss:// URL (Rift
# follows the change by itself); Ctrl-C stops the tunnel and the container
# and puts the previous value back.
#
# A quick tunnel gets a new random URL on every start, and a phone keeps the
# relay URL from its pairing link: pair the phone again (Settings › Remote
# access › Pair a phone) after each start. The phone must reach it while
# Rift is running with remote_access = true on this machine.
set -euo pipefail
cd "$(dirname "$0")/.."

IMAGE=rift-relay
NAME=rift-relay-local
PORT=8790

for tool in docker cloudflared; do
    if ! command -v "$tool" >/dev/null; then
        echo "relay-local: $tool is not installed" >&2
        exit 1
    fi
done
if ! docker info >/dev/null 2>&1; then
    echo "relay-local: the docker daemon is not running" >&2
    exit 1
fi

if [[ "${1:-}" == "--rebuild" ]] || ! docker image inspect "$IMAGE" >/dev/null 2>&1; then
    echo "relay-local: building $IMAGE"
    docker build -f packaging/relay.Dockerfile -t "$IMAGE" .
fi

# config.toml as Rift finds it: $XDG_CONFIG_HOME or ~/.config, then rift/,
# else a legacy shift/ or runway/ directory (rift_core::config::app_dir)
root="${XDG_CONFIG_HOME:-$HOME/.config}"
dir="$root/rift"
if [[ ! -d "$dir" ]]; then
    for legacy in shift runway; do
        if [[ -d "$root/$legacy" ]]; then
            dir="$root/$legacy"
            break
        fi
    done
fi
config="$dir/config.toml"
mkdir -p "$dir"
touch "$config"
previous="$(awk '/^\[/ { exit } /^remote_relay[[:space:]]*=/ { print; exit }' "$config")"

# set the top-level `remote_relay` line: replace it, or insert it before the
# first [table] (a key after a table header would belong to that table)
set_relay() {
    local line="$1" tmp
    tmp="$(mktemp "$config.XXXXXX")"
    awk -v line="$line" '
        BEGIN { done = 0; top = 1 }
        /^\[/ && top { top = 0; if (!done && line != "") { print line; done = 1 } }
        top && /^remote_relay[[:space:]]*=/ { if (!done && line != "") { print line; done = 1 }; next }
        { print }
        END { if (!done && line != "") print line }
    ' "$config" >"$tmp"
    mv "$tmp" "$config"
}

tunnel_pid=""
log="$(mktemp -t rift-relay-tunnel)"
cleanup() {
    trap - EXIT INT TERM
    echo
    echo "relay-local: stopping"
    if [[ -n "$tunnel_pid" ]]; then
        kill "$tunnel_pid" 2>/dev/null || true
        wait "$tunnel_pid" 2>/dev/null || true
    fi
    docker stop "$NAME" >/dev/null 2>&1 || true
    set_relay "$previous"
    rm -f "$log"
}
trap cleanup EXIT INT TERM

docker rm -f "$NAME" >/dev/null 2>&1 || true
# only this machine reaches the port; the tunnel is the public way in, and
# Cloudflare appends the phone's address to X-Forwarded-For, so the rate
# limits count real clients
docker run -d --rm --name "$NAME" -p "127.0.0.1:$PORT:8790" "$IMAGE" \
    --listen 0.0.0.0:8790 --trust-forwarded >/dev/null

cloudflared tunnel --no-autoupdate --url "http://127.0.0.1:$PORT" >"$log" 2>&1 &
tunnel_pid=$!

url=""
for _ in $(seq 1 60); do
    url="$(grep -Eo 'https://[a-z0-9-]+\.trycloudflare\.com' "$log" | head -n 1 || true)"
    [[ -n "$url" ]] && break
    if ! kill -0 "$tunnel_pid" 2>/dev/null; then
        echo "relay-local: cloudflared exited:" >&2
        cat "$log" >&2
        exit 1
    fi
    sleep 0.5
done
if [[ -z "$url" ]]; then
    echo "relay-local: no tunnel URL after 30 s:" >&2
    cat "$log" >&2
    exit 1
fi

relay="wss://${url#https://}"
set_relay "remote_relay = \"$relay\""
echo "relay-local: relay at $relay"
echo "relay-local: wrote remote_relay to $config"
if ! grep -Eq '^remote_access[[:space:]]*=[[:space:]]*true' "$config"; then
    echo "relay-local: remote_access is off: turn on Settings › Remote access"
fi
echo "relay-local: pair the phone again, then use it anywhere. Ctrl-C stops."
wait "$tunnel_pid"
