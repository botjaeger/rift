#!/usr/bin/env python3
"""Write latest.json, the manifest Shift's updater reads (crates/shift-core/src/update.rs).

usage: scripts/release-manifest.py VERSION ARTIFACT_DIR [-o latest.json]
                                   [--base-url URL] [--allow-missing]
       scripts/release-manifest.py --verify latest.json ARTIFACT_DIR
       scripts/release-manifest.py --self-test

ARTIFACT_DIR is searched recursively for this version's assets:

  Shift-VERSION-arm64.dmg         -> macos-aarch64
  Shift-VERSION-x86_64.dmg        -> macos-x86_64
  Shift-VERSION-x86_64.AppImage   -> linux-x86_64

Each asset's URL is BASE_URL/<file name>; BASE_URL defaults to the public
release's download folder,
https://github.com/botjaeger/shift-releases/releases/download/vVERSION.
Fails when CHANGELOG.md has no section for VERSION, or (without
--allow-missing) when an asset is missing. --verify checks an existing
manifest against the files: every asset present under ARTIFACT_DIR with the
recorded SHA-256, and its URL in the version's public release.
"""
import argparse
import datetime
import hashlib
import json
import re
import sys
import tempfile
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]
RELEASES = "https://github.com/botjaeger/shift-releases/releases"
ASSETS = {
    "macos-aarch64": ("Shift-{v}-arm64.dmg", "dmg"),
    "macos-x86_64": ("Shift-{v}-x86_64.dmg", "dmg"),
    "linux-x86_64": ("Shift-{v}-x86_64.AppImage", "appimage"),
}


def protocol(ipc_rs: Path) -> int:
    m = re.search(r"pub const PROTOCOL_VERSION: u32 = (\d+);", ipc_rs.read_text())
    if not m:
        raise SystemExit(f"error: no PROTOCOL_VERSION in {ipc_rs}")
    return int(m[1])


def changelog_section(changelog: str, version: str):
    """(date, notes) of VERSION's section, or None."""
    heading = re.compile(r"^## v?" + re.escape(version) + r"(?:\s+—\s+(\S+))?\s*$", re.M)
    m = heading.search(changelog)
    if not m:
        return None
    rest = changelog[m.end():]
    end = re.search(r"^## ", rest, re.M)
    body = rest[: end.start()] if end else rest
    # the summary paragraph before the first ### heading, else the first items
    summary = re.split(r"^### ", body, maxsplit=1, flags=re.M)[0].strip()
    if not summary:
        items = [l for l in body.splitlines() if l.startswith("- ")][:3]
        summary = "\n".join(items)
    return m[1], " ".join(summary.split())


def sha256(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def manifest(version, artifacts: Path, base_url=None, allow_missing=False,
             changelog=None, ipc_rs=None):
    version = version.lstrip("v")
    if not re.fullmatch(r"\d+\.\d+\.\d+(-[0-9A-Za-z.]+)?", version):
        raise SystemExit(f"error: {version!r} is not x.y.z")
    changelog = changelog if changelog is not None else (REPO / "CHANGELOG.md").read_text()
    section = changelog_section(changelog, version)
    if section is None:
        raise SystemExit(f"error: CHANGELOG.md has no section for {version}")
    date, notes = section
    base_url = (base_url or f"{RELEASES}/download/v{version}").rstrip("/")
    files = {p.name: p for p in artifacts.rglob("*") if p.is_file()}
    assets = {}
    for key, (pattern, kind) in ASSETS.items():
        name = pattern.format(v=version)
        if name not in files:
            if allow_missing:
                continue
            raise SystemExit(f"error: {name} not found under {artifacts}")
        assets[key] = {"url": f"{base_url}/{name}", "sha256": sha256(files[name]), "kind": kind}
    if not assets:
        raise SystemExit(f"error: no assets for {version} under {artifacts}")
    return {
        "version": version,
        "date": date or datetime.date.today().isoformat(),
        "protocol": protocol(ipc_rs or REPO / "crates/shift-pty/src/ipc.rs"),
        "notes": notes,
        "url": f"{RELEASES}/tag/v{version}",
        "assets": assets,
    }


def verify(path: Path, artifacts: Path, base_url=None):
    """Raise SystemExit unless every asset of the manifest at PATH matches a
    file under ARTIFACTS; returns the manifest."""
    m = json.loads(path.read_text())
    version = m.get("version", "")
    base_url = (base_url or f"{RELEASES}/download/v{version}").rstrip("/")
    files = {p.name: p for p in artifacts.rglob("*") if p.is_file()}
    if not m.get("assets"):
        raise SystemExit(f"error: {path} lists no assets")
    for key, a in m["assets"].items():
        name = a["url"].rsplit("/", 1)[-1]
        if a["url"] != f"{base_url}/{name}":
            raise SystemExit(f"error: {key}: {a['url']} is not under {base_url}")
        if name not in files:
            raise SystemExit(f"error: {key}: {name} not found under {artifacts}")
        if sha256(files[name]) != a["sha256"]:
            raise SystemExit(f"error: {key}: {name} does not match its sha256")
    return m


def self_test():
    log = ("# Changelog\n\nPreamble.\n\n## 1.2.0 — 2026-11-01\n\nFaster\nblocks.\n\n"
           "### New\n\n- **A** — b\n\n## 1.1.0 — 2026-10-01\n\n### Fixed\n\n- **X** — y\n")
    assert changelog_section(log, "1.2.0") == ("2026-11-01", "Faster blocks.")
    assert changelog_section(log, "1.1.0") == ("2026-10-01", "- **X** — y")
    assert changelog_section(log, "1.3.0") is None
    with tempfile.TemporaryDirectory() as d:
        d = Path(d)
        ipc = d / "ipc.rs"
        ipc.write_text("pub const PROTOCOL_VERSION: u32 = 7;\n")
        (d / "mac").mkdir()
        (d / "mac/Shift-1.2.0-arm64.dmg").write_bytes(b"abc")
        (d / "Shift-1.2.0-x86_64.dmg").write_bytes(b"")
        (d / "Shift-1.2.0-x86_64.AppImage").write_bytes(b"x")
        (d / "Shift-1.1.0-arm64.dmg").write_bytes(b"old")
        m = manifest("v1.2.0", d, changelog=log, ipc_rs=ipc)
        assert m["version"] == "1.2.0" and m["protocol"] == 7, m
        assert m["date"] == "2026-11-01" and m["notes"] == "Faster blocks.", m
        assert m["url"] == f"{RELEASES}/tag/v1.2.0", m
        a = m["assets"]["macos-aarch64"]
        assert a == {"url": f"{RELEASES}/download/v1.2.0/Shift-1.2.0-arm64.dmg",
                     "sha256": "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
                     "kind": "dmg"}, a
        assert m["assets"]["linux-x86_64"]["kind"] == "appimage", m
        # a version without a changelog section (even with its asset), or a
        # missing asset, fails
        (d / "Shift-1.3.0-arm64.dmg").write_bytes(b"new")
        for version, allow, why in [("1.3.0", True, "CHANGELOG"), ("1.1.0", False, "not found")]:
            try:
                manifest(version, d, allow_missing=allow, changelog=log, ipc_rs=ipc)
            except SystemExit as e:
                assert why in str(e), e
            else:
                raise AssertionError(f"{version} should fail")
        m = manifest("1.1.0", d, allow_missing=True, base_url="file:///x/", changelog=log, ipc_rs=ipc)
        assert list(m["assets"]) == ["macos-aarch64"], m
        assert m["assets"]["macos-aarch64"]["url"] == "file:///x/Shift-1.1.0-arm64.dmg", m
        # --verify: the written manifest matches; a changed file does not
        out = d / "latest.json"
        out.write_text(json.dumps(manifest("1.2.0", d, changelog=log, ipc_rs=ipc)))
        assert verify(out, d)["version"] == "1.2.0"
        (d / "Shift-1.2.0-x86_64.AppImage").write_bytes(b"tampered")
        try:
            verify(out, d)
        except SystemExit as e:
            assert "sha256" in str(e), e
        else:
            raise AssertionError("a tampered asset should fail --verify")
    # the repository's own files parse
    assert protocol(REPO / "crates/shift-pty/src/ipc.rs") >= 1
    print("release-manifest self-test passed")


def main():
    if sys.argv[1:] == ["--self-test"]:
        self_test()
        return
    if sys.argv[1:2] == ["--verify"] and len(sys.argv) == 4:
        m = verify(Path(sys.argv[2]), Path(sys.argv[3]))
        print(f"{sys.argv[2]}: {m['version']}, {len(m['assets'])} assets verified")
        return
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("version")
    ap.add_argument("artifacts", type=Path)
    ap.add_argument("-o", "--output", type=Path, default=Path("latest.json"))
    ap.add_argument("--base-url")
    ap.add_argument("--allow-missing", action="store_true")
    a = ap.parse_args()
    m = manifest(a.version, a.artifacts, a.base_url, a.allow_missing)
    a.output.write_text(json.dumps(m, indent=2) + "\n")
    print(f"wrote {a.output}: {m['version']}, {', '.join(m['assets'])}")


if __name__ == "__main__":
    main()
