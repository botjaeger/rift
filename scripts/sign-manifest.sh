#!/usr/bin/env bash
# Sign a release manifest for Rift's updater: writes latest.json.sig (base64
# of the Ed25519 signature over latest.json's exact bytes) next to it, then
# checks it against the public keys the app trusts (RELEASE_KEYS in
# crates/rift-core/src/update.rs) so a wrong key is caught before publishing.
#
# Usage: scripts/sign-manifest.sh path/to/latest.json
#        scripts/sign-manifest.sh --verify path/to/latest.json
#   --verify  only check an existing latest.json.sig (needs no private key)
#
# The private key lives only on the maintainer's machine, at
# $RIFT_SIGNING_KEY (default ~/.config/rift-release/signing-key.pem); it
# must be readable by its owner only. CI never signs. Needs OpenSSL 3 or
# newer ($OPENSSL, else `openssl` on PATH, else Homebrew's openssl@3):
# macOS's LibreSSL has no Ed25519.
set -euo pipefail
cd "$(dirname "$0")/.."
# shellcheck source=packaging/lib.sh
. packaging/lib.sh

KEY="${RIFT_SIGNING_KEY:-$HOME/.config/rift-release/signing-key.pem}"
VERIFY_ONLY=0
MANIFEST=""
while [ $# -gt 0 ]; do
    case "$1" in
        --verify)  VERIFY_ONLY=1; shift ;;
        -h|--help) rift_usage; exit 0 ;;
        -*)        rift_die "unknown option: $1" ;;
        *)         MANIFEST="$1"; shift ;;
    esac
done
[ -n "$MANIFEST" ] || rift_die "usage: scripts/sign-manifest.sh [--verify] path/to/latest.json"
[ -f "$MANIFEST" ] || rift_die "$MANIFEST does not exist"
SIG="$MANIFEST.sig"

# an OpenSSL that signs Ed25519 (3.0 or newer, not LibreSSL)
OSSL=""
for c in "${OPENSSL:-}" openssl /opt/homebrew/opt/openssl@3/bin/openssl \
    /usr/local/opt/openssl@3/bin/openssl; do
    if [ -z "$c" ] || ! command -v "$c" >/dev/null 2>&1; then
        continue
    fi
    case "$("$c" version 2>/dev/null)" in
        "OpenSSL "[3-9].* | "OpenSSL "[1-9][0-9]*) OSSL="$c"; break ;;
    esac
done
[ -n "$OSSL" ] || rift_die "OpenSSL 3 or newer is required (brew install openssl@3; or set OPENSSL)"

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

if [ "$VERIFY_ONLY" = 0 ]; then
    [ -f "$KEY" ] || rift_die "no signing key at $KEY (see docs/development.md, Releases)"
    if [ "$(uname -s)" = Darwin ]; then
        MODE="$(stat -f %Lp "$KEY")"
    else
        MODE="$(stat -c %a "$KEY")"
    fi
    case "$MODE" in
        400 | 600) ;;
        *) rift_die "$KEY is mode $MODE: the signing key must be readable by its owner only (chmod 600)" ;;
    esac
    "$OSSL" pkeyutl -sign -rawin -inkey "$KEY" -in "$MANIFEST" -out "$TMP/sig.bin" ||
        rift_die "signing failed"
    { "$OSSL" base64 -A -in "$TMP/sig.bin"; echo; } >"$SIG.tmp"
    mv "$SIG.tmp" "$SIG"
fi
[ -f "$SIG" ] || rift_die "$SIG does not exist"
"$OSSL" base64 -d -A -in "$SIG" -out "$TMP/check.bin" || rift_die "$SIG is not base64"

# the trusted keys: the base64 strings of RELEASE_KEYS in update.rs
KEYS="$(awk '/^pub const RELEASE_KEYS/ { on = 1 }
             on { while (match($0, /"[A-Za-z0-9+\/=]+"/)) {
                      print substr($0, RSTART + 1, RLENGTH - 2); $0 = substr($0, RSTART + RLENGTH) } }
             on && /;/ { exit }' crates/rift-core/src/update.rs)"
[ -n "$KEYS" ] || rift_die "no RELEASE_KEYS found in crates/rift-core/src/update.rs"
OK=""
n=0
for k in $KEYS; do
    n=$((n + 1))
    # SubjectPublicKeyInfo DER: the fixed Ed25519 prefix, then the raw key
    printf '\060\052\060\005\006\003\053\145\160\003\041\000' >"$TMP/pub$n.der"
    printf '%s' "$k" | "$OSSL" base64 -d -A >>"$TMP/pub$n.der"
    if "$OSSL" pkeyutl -verify -rawin -pubin -keyform DER -inkey "$TMP/pub$n.der" \
        -in "$MANIFEST" -sigfile "$TMP/check.bin" >/dev/null 2>&1; then
        OK="$k"
        break
    fi
done
if [ -z "$OK" ]; then
    [ "$VERIFY_ONLY" = 1 ] || rm -f "$SIG"
    rift_die "$SIG does not verify against any RELEASE_KEYS key in update.rs (wrong signing key?)"
fi
echo "$SIG verifies with release key $OK"
