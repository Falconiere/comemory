# The comemory CLI as a container image: one static-ish glibc binary on a slim
# Debian base, entrypointed so `docker run ghcr.io/<owner>/comemory search foo`
# behaves like the CLI does on a laptop. Contract + usage: docs/container-image.md.
#
# Multi-arch: linux/amd64 and linux/arm64, the same two Linux targets
# `dist.targets` in Cargo.toml already ships tarballs for. CI compiles each arch
# on a NATIVE runner (.github/workflows/release-image.yml) — this file assumes
# no cross-compilation. A QEMU-emulated build of this dependency tree is
# painfully slow: `rusqlite` (bundled SQLite), `sqlite-vec` and `git2`
# (vendored libgit2) are all C, compiled from source on every build.
#
# The builder's Debian suite is pinned to the runtime's (bookworm) so the
# binary never links a newer glibc than the runtime image provides.

FROM rust:1.95-bookworm AS builder
WORKDIR /src

# The whole tree, because Cargo.toml declares [[bench]] and [lib] targets whose
# paths must resolve before cargo will even parse the manifest. .dockerignore
# keeps target/ and .git/ out, which is where the weight actually is.
COPY . .

# --locked pins the committed Cargo.lock, exactly as release.yml's dist build
# does — an image must not silently resolve a different dependency graph than
# the tarballs published from the same tag.
RUN cargo build --release --locked --bin comemory

FROM debian:bookworm-slim AS runtime

ARG COMEMORY_VERSION

# No `org.opencontainers.image.source` here on purpose. release-image.yml sets
# it from `github.repository`, so it stays correct on a fork or after a rename;
# a literal here would either duplicate that or quietly contradict it, and it is
# the label GHCR uses to link the package to a repository.
LABEL org.opencontainers.image.title="comemory" \
  org.opencontainers.image.description="Agentic dev memory + code-aware semantic search via a two-layer property graph." \
  org.opencontainers.image.licenses="MIT"

# ca-certificates only: `git2` is built with default-features off (no OpenSSL)
# and there is no in-process embedder, so nothing else here reaches the network.
RUN apt-get update \
  && apt-get install -y --no-install-recommends ca-certificates \
  && rm -rf /var/lib/apt/lists/*

COPY --from=builder /src/target/release/comemory /usr/local/bin/comemory

# Fails the build rather than publishing an image whose binary disagrees with
# the tag that triggered it. Skipped when the arg is absent, so a bare
# `docker build .` still works locally.
RUN set -eu; \
  comemory --version; \
  if [ -n "${COMEMORY_VERSION:-}" ]; then \
    comemory --version | grep -qx "comemory ${COMEMORY_VERSION}" \
      || { echo "binary is not ${COMEMORY_VERSION}: $(comemory --version)" >&2; exit 1; }; \
  fi

# Non-root by default. The uid is fixed and high so it cannot collide with a
# real account on a host that bind-mounts into the container; `docker run
# --user "$(id -u):$(id -g)"` is the documented escape hatch when a mounted
# data directory is owned by someone else (docs/container-image.md).
RUN useradd --uid 10001 --create-home --shell /usr/sbin/nologin comemory \
  && mkdir -p /data \
  && chown comemory:comemory /data

# Markdown files plus comemory.db live here — the one path worth persisting.
# COMEMORY_DATA_DIR is what the CLI reads, so a caller can still point it
# elsewhere without rebuilding.
ENV COMEMORY_DATA_DIR=/data
VOLUME /data

USER comemory
WORKDIR /work

ENTRYPOINT ["comemory"]
CMD ["--help"]
