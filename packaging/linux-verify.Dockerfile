# Headless Linux verification image for Shift.
#
# Builds the release binary and drives it through `shift-terminal --script` with no
# GPU and no display: Xvfb provides the X server, Mesa's lavapipe provides a
# software Vulkan device, so `shot` still goes through the real wgpu pipeline.
#
#   docker build -t shift-linux-verify -f packaging/linux-verify.Dockerfile packaging
#   docker run --rm \
#     -v "$PWD:/src" -v "$PWD/target/linux-out:/out" \
#     -e CARGO_TARGET_DIR=/src/target/linux-docker \
#     -e XDG_RUNTIME_DIR=/tmp/xdg \
#     shift-linux-verify bash -c '
#       cargo build --release --locked --manifest-path /src/Cargo.toml &&
#       mkdir -p /tmp/xdg && chmod 700 /tmp/xdg &&
#       xvfb-run -s "-screen 0 1400x900x24" env \
#         SHIFT_NO_NOTIFY=1 RUST_LOG=info WGPU_BACKEND=vulkan \
#         /src/target/linux-docker/release/shift-terminal --script /src/script.txt'
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
    && rm -rf /var/lib/apt/lists/*

ENV WINIT_UNIX_BACKEND=x11 \
    SHIFT_NO_NOTIFY=1
