#!/usr/bin/env bash
# Build a release's artifacts and latest.json on the maintainer's Mac: the
# same packages as .github/workflows/release.yml, for when GitHub Actions
# cannot run it. Publish them with
#   scripts/publish-release.sh vX.Y.Z --dist target/release-local/vX.Y.Z
#
# Usage: scripts/release-local.sh vX.Y.Z
#
# Builds a clean checkout of the tag (a temporary git worktree, so the
# working tree may hold anything): the two
# macOS targets natively (Intel cross-compiled, as in CI), Linux x86_64 in
# docker (packaging/linux-verify.Dockerfile on linux/amd64, emulated on Apple
# silicon). Run scripts/gates.sh all on that commit first: this builds, it
# does not check.
set -euo pipefail
cd "$(dirname "$0")/.."
# shellcheck source=packaging/lib.sh
. packaging/lib.sh

TAG="${1:-}"
[[ $TAG =~ ^v[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?$ ]] || rift_die "usage: scripts/release-local.sh vX.Y.Z"
[ "$(uname -s)" = Darwin ] || rift_die "macOS only (the DMGs need hdiutil)"
command -v docker >/dev/null || rift_die "docker is not installed"
VERSION="$(rift_version)"
[ "v$VERSION" = "$TAG" ] || rift_die "Cargo.toml is $VERSION, not $TAG"
git rev-parse -q --verify "refs/tags/$TAG" >/dev/null || rift_die "no tag $TAG"
[ "$(git show "$TAG:Cargo.toml" | grep -m1 "^version = ")" = "version = \"$VERSION\"" ] ||
    rift_die "$TAG's Cargo.toml is not $VERSION"

DIST="$PWD/target/release-local/$TAG"
SRC="$PWD/target/release-local/src"
rm -rf "$DIST"
mkdir -p "$DIST"
git worktree remove --force "$SRC" 2>/dev/null || rm -rf "$SRC"
git worktree add --detach "$SRC" "$TAG"
trap 'git worktree remove --force "$SRC"' EXIT
# the tag's sources, this checkout's packaging (a packaging fix made after the
# tag still applies to it)
cp -R packaging/. "$SRC/packaging/"
cp scripts/release-manifest.py "$SRC/scripts/"
cd "$SRC"
unset CARGO_TARGET_DIR

for pair in aarch64-apple-darwin:arm64 x86_64-apple-darwin:x86_64; do
    target="${pair%%:*}" arch="${pair##*:}"
    rustup target add "$target" >/dev/null
    rm -f target/*.dmg target/*.tar.gz
    packaging/make-app.sh --target "$target"
    packaging/make-dmg.sh --arch "$arch"
    packaging/make-tarball.sh --target "$target" --no-build
    mkdir -p "$DIST/macos-$arch"
    mv target/*.dmg target/*.tar.gz "$DIST/macos-$arch/"
done

# its own target dir: an amd64 build must not reuse the arm64 linux gate's
LINUX_TARGET=target/linux-amd64
docker build -q --platform linux/amd64 -t rift-linux-verify-amd64 \
    -f packaging/linux-verify.Dockerfile packaging >/dev/null
rm -f "$LINUX_TARGET"/*.deb "$LINUX_TARGET"/*.AppImage "$LINUX_TARGET"/*.tar.gz
docker run --rm --init --platform linux/amd64 -v "$PWD:/src" -w /src \
    -v rift-cargo-registry-amd64:/usr/local/cargo/registry \
    -e CARGO_TARGET_DIR="/src/$LINUX_TARGET" \
    rift-linux-verify-amd64 bash -c "
        cargo build --release --locked &&
        packaging/make-deb.sh --no-build --target-dir /src/$LINUX_TARGET &&
        packaging/make-appimage.sh --no-build --target-dir /src/$LINUX_TARGET &&
        packaging/make-tarball.sh --no-build --target-dir /src/$LINUX_TARGET"
mkdir -p "$DIST/linux-x86_64"
mv "$LINUX_TARGET"/*.deb "$LINUX_TARGET"/*.AppImage "$LINUX_TARGET"/*.tar.gz "$DIST/linux-x86_64/"

mkdir -p "$DIST/latest-json"
python3 scripts/release-manifest.py "$VERSION" "$DIST" -o "$DIST/latest-json/latest.json"
python3 scripts/release-manifest.py --verify "$DIST/latest-json/latest.json" "$DIST"
find "$DIST" -type f -print0 | xargs -0 ls -lh
echo "next: scripts/publish-release.sh $TAG --dist ${DIST#"$PWD"/}"
