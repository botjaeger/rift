# The Rift relay (`rift-relay`): splices phones to Rift remote gateways
# outside the local network. It only ever forwards Noise ciphertext and
# holds no key (docs/remote-protocol.md § Relay). Only the relay is built
# (crate rift-remote, `--no-default-features`: std + tungstenite, no GUI
# libraries, no TLS — the proxy in front does TLS).
#
#   docker build -f packaging/relay.Dockerfile -t rift-relay .
#   docker run --rm -p 8790:8790 rift-relay
#
# It serves plain ws:// on 8790. Put a TLS reverse proxy in front and give
# gateways (`remote_relay = "wss://relay.example.com"`) and phones the wss://
# URL. With Caddy (automatic HTTPS), docker-compose.yml:
#
#   services:
#     relay:
#       image: rift-relay
#       # the rate limit counts the client address Caddy forwards
#       command: ["--listen", "0.0.0.0:8790", "--trust-forwarded"]
#       restart: unless-stopped
#     caddy:
#       image: caddy:2
#       ports: ["80:80", "443:443"]
#       command: caddy reverse-proxy --from relay.example.com --to relay:8790
#       volumes: ["caddy-data:/data"]
#       restart: unless-stopped
#   volumes:
#     caddy-data:
#
# Only `--trust-forwarded` behind your own proxy: otherwise clients choose
# the address they are rate-limited by.
FROM rust:1.98.0-bookworm AS build
WORKDIR /src
# the workspace manifests and the vendored crates its [patch] names
# (relay.Dockerfile.dockerignore keeps everything else out)
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates crates
COPY vendor vendor
RUN cargo build --release --locked -p rift-remote --no-default-features --bin rift-relay \
 && strip target/release/rift-relay

FROM debian:bookworm-slim
RUN useradd --system --uid 10001 --no-create-home --shell /usr/sbin/nologin relay
COPY --from=build /src/target/release/rift-relay /usr/local/bin/rift-relay
USER relay
EXPOSE 8790
ENTRYPOINT ["/usr/local/bin/rift-relay"]
CMD ["--listen", "0.0.0.0:8790"]
