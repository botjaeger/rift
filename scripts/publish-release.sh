#!/usr/bin/env bash
# Publish a release to the public botjaeger/rift-releases repository from
# the artifacts of the Release workflow run for that tag (in the private
# botjaeger/rift). Run by the maintainer with their own `gh` login.
#
# Usage: scripts/publish-release.sh vX.Y.Z [--run RUN_ID] [--draft]
#   --run    use that workflow run instead of the newest successful one
#   --draft  create the release as a draft (the updater ignores drafts)
# A version with a pre-release suffix (v1.2.0-rc.1) is published as a
# GitHub pre-release.
#
# Refuses when latest.json names another version than the tag, when an
# asset does not match its recorded SHA-256, when two artifacts share a file
# name, when CHANGELOG.md has no section for the version, or when the
# release already exists. Signs latest.json with the maintainer's offline
# key (scripts/sign-manifest.sh) and uploads latest.json.sig with it; the
# signature is checked against the app's RELEASE_KEYS before publishing.
set -euo pipefail
cd "$(dirname "$0")/.."
# shellcheck source=packaging/lib.sh
. packaging/lib.sh

SOURCE_REPO=botjaeger/rift
PUBLIC_REPO=botjaeger/rift-releases

TAG=""
RUN=""
DRAFT=()
while [ $# -gt 0 ]; do
    case "$1" in
        --run)     RUN="${2:?--run needs a run id}"; shift 2 ;;
        --draft)   DRAFT=(--draft); shift ;;
        -h|--help) rift_usage; exit 0 ;;
        v*)        TAG="$1"; shift ;;
        *)         rift_die "unknown argument: $1 (usage: scripts/publish-release.sh vX.Y.Z)" ;;
    esac
done
[[ $TAG =~ ^v[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?$ ]] ||
    rift_die "usage: scripts/publish-release.sh vX.Y.Z [--run RUN_ID] [--draft]"
VERSION="${TAG#v}"
# bash 3.2 (macOS /bin/bash) treats an empty array as unset under `set -u`:
# expand FLAGS as ${FLAGS[@]+"${FLAGS[@]}"}
FLAGS=(${DRAFT[@]+"${DRAFT[@]}"})
case "$VERSION" in
    *-*) FLAGS+=(--prerelease) ;;
esac
command -v gh >/dev/null || rift_die "gh is not installed"
command -v python3 >/dev/null || rift_die "python3 is not installed"

if gh release view "$TAG" -R "$PUBLIC_REPO" >/dev/null 2>&1; then
    rift_die "$PUBLIC_REPO already has a release $TAG"
fi

# the release notes: that version's CHANGELOG.md section (without its heading)
NOTES="$(awk -v v="$VERSION" '
    $0 ~ "^## v?" v "( |$)" { on = 1; next }
    on && /^## /             { exit }
    on                       { print }
' CHANGELOG.md)"
[ -n "${NOTES//[[:space:]]/}" ] || rift_die "CHANGELOG.md has no section for $VERSION"

if [ -z "$RUN" ]; then
    RUN="$(gh run list -R "$SOURCE_REPO" --workflow release.yml --limit 50 \
        --json databaseId,headBranch,conclusion \
        --jq "[.[] | select(.headBranch == \"$TAG\" and .conclusion == \"success\")][0].databaseId // empty")"
    [ -n "$RUN" ] || rift_die "no successful Release workflow run for $TAG in $SOURCE_REPO"
fi
echo "release workflow run $RUN"

DIST="$(mktemp -d)"
trap 'rm -rf "$DIST"' EXIT
gh run download "$RUN" -R "$SOURCE_REPO" -D "$DIST"

MANIFEST="$(find "$DIST" -type f -name latest.json | head -n 1)"
[ -n "$MANIFEST" ] || rift_die "run $RUN has no latest.json artifact"
python3 scripts/release-manifest.py --verify "$MANIFEST" "$DIST"
GOT="$(python3 -c 'import json, sys; print(json.load(open(sys.argv[1]))["version"])' "$MANIFEST")"
[ "$GOT" = "$VERSION" ] || rift_die "latest.json is for $GOT, not $TAG"
# the updater refuses a manifest without a valid signature: sign it here
# (the key never reaches CI), check it, and publish latest.json.sig with it
[ ! -e "$MANIFEST.sig" ] || rift_die "run $RUN already carries a latest.json.sig"
scripts/sign-manifest.sh "$MANIFEST"
scripts/sign-manifest.sh --verify "$MANIFEST"

FILES=()
while IFS= read -r -d '' f; do
    FILES+=("$f")
done < <(find "$DIST" -type f -print0 | sort -z)
# a release asset is named by its file name: two artifacts with one name
# (e.g. both macOS tarballs named for the build host) would collide
DUPES="$(printf '%s\n' "${FILES[@]##*/}" | sort | uniq -d)"
[ -z "$DUPES" ] || rift_die "artifacts share a file name: $(printf '%s' "$DUPES" | tr '\n' ' ')"
printf '%s\n' "${FILES[@]##*/}" | grep -qx 'latest.json.sig' ||
    rift_die "latest.json.sig is not among the release files"
printf 'publishing to %s %s:\n' "$PUBLIC_REPO" "$TAG"
printf '  %s\n' "${FILES[@]##*/}"

printf '%s\n' "$NOTES" >"$DIST/notes.md"
gh release create "$TAG" -R "$PUBLIC_REPO" \
    --title "Rift $VERSION" \
    --notes-file "$DIST/notes.md" \
    ${FLAGS[@]+"${FLAGS[@]}"} \
    "${FILES[@]}"
echo "published https://github.com/$PUBLIC_REPO/releases/tag/$TAG"
