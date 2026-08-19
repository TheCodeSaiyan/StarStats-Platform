# syntax=docker/dockerfile:1.7
# Multi-stage build for the Axum API server.
# Final image: ~30 MB on debian:bookworm-slim with the compiled binary.

FROM rust:1.88-slim-bookworm@sha256:38bc5a86d998772d4aec2348656ed21438d20fcdce2795b56ca434cf21430d89 AS builder
WORKDIR /build

RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config libssl-dev ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Cache layer: copy only manifests, build a stub workspace, populate dep cache.
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates/starstats-core/Cargo.toml crates/starstats-core/
COPY crates/starstats-server/Cargo.toml crates/starstats-server/
COPY crates/starstats-client/Cargo.toml crates/starstats-client/

RUN mkdir -p crates/starstats-core/src crates/starstats-server/src/bin crates/starstats-client/src \
    && echo "" > crates/starstats-core/src/lib.rs \
    && echo "fn main() {}" > crates/starstats-server/src/main.rs \
    && echo "fn main() {}" > crates/starstats-server/src/bin/openapi.rs \
    && echo "fn main() {}" > crates/starstats-client/src/main.rs \
    && cargo build --release -p starstats-server \
    && rm -rf crates/starstats-core/src crates/starstats-server/src crates/starstats-client/src

# Real source. The previous comment claimed "touch invalidates the stub
# artefacts so cargo recompiles" — it did not. BuildKit COPY preserves
# the source tree's mtime from the build context (often older than the
# fingerprint files emitted by the stub build above), so cargo's
# mtime-based incremental detection saw the sources as "not newer than
# what was already built" and the stub `fn main() {}` binary survived
# to the final image as a 336KB EXIT=0 silent-on-stdout no-op (caught
# only at runtime on dockerprime). Two belt-and-braces fixes:
#
# 1. Delete the stub-built binary + per-crate fingerprint files so
#    cargo CAN'T reuse them regardless of mtime semantics.
# 2. `find ... -exec touch {} +` the real sources so their mtime is
#    bumped past the fingerprints from the stub build — second layer
#    of defence if (1) ever misses an artefact.
# 3. Size-gate the resulting binary at the end of this RUN: a real
#    starstats-server release build is ~30MB; anything under 5MB is
#    almost certainly the stub regressing. Fail the build loudly so
#    a busted image never reaches the registry again.
COPY crates/starstats-core crates/starstats-core
COPY crates/starstats-server crates/starstats-server

RUN rm -f target/release/starstats-server \
          target/release/deps/starstats_server-* \
          target/release/deps/libstarstats_core-* \
    && rm -rf target/release/.fingerprint/starstats-server-* \
              target/release/.fingerprint/starstats-core-* \
    && find crates/starstats-core/src crates/starstats-server/src -type f -exec touch {} + \
    && cargo build --release -p starstats-server \
    && BIN_SIZE=$(stat -c%s target/release/starstats-server) \
    && echo "starstats-server release binary: ${BIN_SIZE} bytes" \
    && if [ "${BIN_SIZE}" -lt 5000000 ]; then \
         echo "FATAL: binary is ${BIN_SIZE} bytes (< 5MB) — stub-binary regression"; \
         ls -la target/release/starstats-server; \
         exit 1; \
       fi

# Runtime image — minimal, has wget for HEALTHCHECK.
FROM debian:bookworm-slim@sha256:f9c6a2fd2ddbc23e336b6257a5245e31f996953ef06cd13a59fa0a1df2d5c252 AS runtime
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates wget tini \
    && rm -rf /var/lib/apt/lists/*

RUN groupadd --system --gid 65532 starstats \
    && useradd --system --uid 65532 --gid starstats --no-create-home starstats

# Pre-create the writable state dir OWNED BY the runtime user. Docker
# initializes a fresh named volume mounted at this path with the directory's
# ownership + mode from the image, so the non-root starstats user can write
# jwt-key.pem / totp-kek.bin / updater-manifest.json on first boot. Without
# this the image has no such dir, Docker makes the fresh-volume mountpoint
# root:root, and every first write fails with EACCES (os error 13),
# crash-looping the server (surfaced on dockerprime 2026-07-22). 0700 keeps
# the signing-key material owner-only.
RUN mkdir -p /var/lib/starstats \
    && chown starstats:starstats /var/lib/starstats \
    && chmod 700 /var/lib/starstats

COPY --from=builder /build/target/release/starstats-server /usr/local/bin/starstats-server

USER starstats:starstats
EXPOSE 8080
HEALTHCHECK --interval=30s --timeout=5s --start-period=30s --retries=3 \
    CMD wget --quiet --spider http://localhost:8080/healthz || exit 1

ENTRYPOINT ["tini", "--", "/usr/local/bin/starstats-server"]
