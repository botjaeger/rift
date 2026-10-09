# The CI Linux runner's package set (ubuntu-24.04 + packaging/ci-apt.txt), as a
# non-root user. `scripts/gates.sh linux` runs the scripted UI regressions in it,
# so a library or font the runner lacks fails locally first.
FROM ubuntu:24.04

COPY ci-apt.txt /tmp/ci-apt.txt
RUN apt-get update \
    && xargs -a /tmp/ci-apt.txt env DEBIAN_FRONTEND=noninteractive apt-get install -y python3 git \
    && rm -rf /var/lib/apt/lists/* \
    && useradd -m runner
USER runner
