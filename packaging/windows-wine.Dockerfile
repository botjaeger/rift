# Runs Rift's Windows test binaries (cross-compiled on the host for
# x86_64-pc-windows-gnu) under Wine: the `windows` gate (scripts/gates.sh).
# Wine is not Windows — ConPTY, the GPU and windows still need a real
# Windows machine — but the platform layer (processes, files, locks,
# sockets, paths) runs for real.
#
#   docker build --platform linux/amd64 -t rift-wine -f packaging/windows-wine.Dockerfile packaging
#   docker run --rm --platform linux/amd64 -v "$PWD:$PWD" rift-wine \
#     sh "$PWD/scripts/wine-tests.sh" "$PWD"
#
# The repository is mounted at its own path: Wine's Z: drive is `/`, so the
# tests' compiled-in fixture paths resolve.
FROM debian:trixie-slim
RUN apt-get update \
 && apt-get install -y --no-install-recommends wine wine64 ca-certificates unzip \
 && rm -rf /var/lib/apt/lists/*
ENV WINEDEBUG=-all WINEPREFIX=/wine
# first start creates the prefix (slow); do it once in the image
RUN wineboot --init && wineserver --wait
# Windows 10+ ships curl.exe in System32 (the phone gateway's push posts
# run it); Wine does not: the official build, pinned
ADD --checksum=sha256:89cb71930f4242d884e8e349463455932ab1d089b20f4f99318261a3cbff9167 \
    https://curl.se/windows/dl-8.23.0_9/curl-8.23.0_9-win64-mingw.zip /tmp/curl.zip
RUN unzip -j /tmp/curl.zip 'curl-8.23.0_9-win64-mingw/bin/curl.exe' \
      'curl-8.23.0_9-win64-mingw/bin/libcurl-x64.dll' -d /wine/drive_c/windows/system32 \
 && rm /tmp/curl.zip
