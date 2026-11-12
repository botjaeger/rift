# Headless Linux verification image for Rift.
#
# Builds the release binary and drives it through `rift-terminal --script` with no
# GPU and no display: Xvfb provides the X server, Mesa's lavapipe provides a
# software Vulkan device, so `shot` still goes through the real wgpu pipeline.
#
#   docker build -t rift-linux-verify -f packaging/linux-verify.Dockerfile packaging
#   docker run --rm \
#     -v "$PWD:/src" -v "$PWD/target/linux-out:/out" \
#     -e CARGO_TARGET_DIR=/src/target/linux-docker \
#     -e XDG_RUNTIME_DIR=/tmp/xdg \
#     rift-linux-verify bash -c '
#       cargo build --release --locked --manifest-path /src/Cargo.toml &&
#       mkdir -p /tmp/xdg && chmod 700 /tmp/xdg &&
#       xvfb-run -s "-screen 0 1400x900x24" env \
#         RIFT_NO_NOTIFY=1 RUST_LOG=info WGPU_BACKEND=vulkan \
#         /src/target/linux-docker/release/rift-terminal --script /src/script.txt'
#
# Blocks only appear when the shell emits OSC 133: zsh (ZDOTDIR shim) and
# bash (`--rcfile` shim) both work out of the box
FROM rust:1.98.0-bookworm

RUN apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
      # winit / wgpu build deps
      libxkbcommon-dev libwayland-dev libxcb1-dev libxcb-render0-dev \
      libxcb-shape0-dev libxcb-xfixes0-dev pkg-config dpkg-dev file \
      # headless display + software Vulkan (lavapipe)
      xvfb xauth dbus-x11 \
      mesa-vulkan-drivers libvulkan1 vulkan-tools libgl1-mesa-dri libegl1 libgl1 \
      # runtime libs winit dlopens
      libxkbcommon0 libxkbcommon-x11-0 libwayland-client0 \
      libx11-6 libx11-xcb1 libxcursor1 libxi6 libxrandr2 \
      # shells whose integration is exercised
      zsh \
      # the test gate's release-manifest.py --self-test
      python3 \
    && rm -rf /var/lib/apt/lists/*

# PowerShell, whose integration (shell/rift.ps1) the test gate runs: the
# official build, pinned (no ICU: invariant globalization)
ARG TARGETARCH
RUN case "$TARGETARCH" in \
      amd64) a=x64 s=6f6015203c47806c5cc444c19d8ed019695e610fbd948154264bf9ca8e157561 ;; \
      arm64) a=arm64 s=c0159b03e85f44ae1e7697818a011558da6c813d0aae848bf5ac13bf435d8624 ;; \
      *) echo "no PowerShell build for $TARGETARCH" >&2; exit 1 ;; \
    esac \
 && curl -fsSL -o /tmp/pwsh.tgz \
      "https://github.com/PowerShell/PowerShell/releases/download/v7.4.6/powershell-7.4.6-linux-$a.tar.gz" \
 && echo "$s  /tmp/pwsh.tgz" | sha256sum -c - \
 && mkdir -p /opt/pwsh && tar xzf /tmp/pwsh.tgz -C /opt/pwsh && rm /tmp/pwsh.tgz \
 && chmod +x /opt/pwsh/pwsh && ln -s /opt/pwsh/pwsh /usr/local/bin/pwsh

ENV WINIT_UNIX_BACKEND=x11 \
    RIFT_NO_NOTIFY=1 \
    DOTNET_SYSTEM_GLOBALIZATION_INVARIANT=1
