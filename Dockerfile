# syntax=docker/dockerfile:1.7
#
# Photovault image: the server binary plus the web apps (Photos at /, and
# /albums, /cast, /accounts, /embed) on distroless/cc (glibc + libgcc only, no
# shell, no package manager), running as uid 65532. One container serves the
# API and the web apps on one port.
#
#   docker build -t photovault .
#
# Options:
#   --build-arg RUST_IMAGE=rust:1.99-bookworm   (Docker Hub instead of Google's mirror;
#                                                 same for NODE_IMAGE)
#   --secret id=ca,src=/path/ca.crt   (TLS-intercepting proxy; used only while building)
#   --build-arg PHOTOVAULT_COMMIT=$(git rev-parse --short HEAD)   (shown in /photovault/info)
#   --build-arg WEB=prebuilt --build-context webout=path/to/photovault-web
#                                     (use a web build made earlier, e.g. the
#                                      photovault-web CI artifact, instead of building it)
#   --build-arg WEB=none              (server only, no web apps)
#
# The web apps are built with web/scripts/photovault-build.sh, the same
# commands as the photovault-web CI job. The output is the same for every CPU
# architecture, so that stage runs on the build host's platform.
#
# libsodium is compiled from the source bundled in libsodium-sys-stable and
# SQLite is bundled by sqlx, so the binary needs nothing beyond glibc.

ARG RUST_IMAGE=mirror.gcr.io/library/rust:1.99-bookworm
ARG NODE_IMAGE=mirror.gcr.io/library/node:22-bookworm
ARG RUNTIME_IMAGE=gcr.io/distroless/cc-debian12:nonroot
ARG WEB=build

# ---- web apps -------------------------------------------------------------
FROM --platform=$BUILDPLATFORM ${NODE_IMAGE} AS node

FROM --platform=$BUILDPLATFORM ${RUST_IMAGE} AS web-build
# Node 22 from the official image, next to the Rust image's rustup and cargo.
COPY --from=node /usr/local/bin/ /usr/local/bin/
COPY --from=node /usr/local/lib/node_modules/ /usr/local/lib/node_modules/
COPY --from=node /usr/local/include/node/ /usr/local/include/node/
ENV CARGO_TERM_COLOR=never NEXT_TELEMETRY_DISABLED=1 \
    WASM_PACK_VERSION=0.15.1 \
    NEXT_PUBLIC_ENTE_ENDPOINT=""
COPY rust/bindings/wasm/cast/rust-toolchain.toml /tmp/cast-toolchain.toml
RUN --mount=type=secret,id=ca,required=false \
    --mount=type=cache,target=/usr/local/cargo/registry \
    if [ -s /run/secrets/ca ]; then cat /run/secrets/ca >> /etc/ssl/certs/ca-certificates.crt; fi && \
    node --version && npm --version && \
    rustup target add wasm32-unknown-unknown && \
    channel="$(sed -n 's/^channel = "\(.*\)"/\1/p' /tmp/cast-toolchain.toml)" && \
    rustup toolchain install "$channel" --profile minimal \
        --component rust-src --target wasm32-unknown-unknown && \
    cargo install wasm-pack --locked --version "$WASM_PACK_VERSION"
WORKDIR /src
COPY web web
COPY rust rust
RUN --mount=type=secret,id=ca,required=false \
    --mount=type=cache,target=/root/.npm \
    --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/rust/target \
    --mount=type=cache,target=/root/.cache/photovault-build \
    if [ -s /run/secrets/ca ]; then export NODE_EXTRA_CA_CERTS=/run/secrets/ca; fi && \
    PHOTOVAULT_WEB_OUT=/webout web/scripts/photovault-build.sh && \
    test -f /webout/index.html && test -f /webout/albums/index.html

FROM ${RUST_IMAGE} AS web-prebuilt
COPY --from=webout . /webout/
RUN test -f /webout/index.html

FROM ${RUST_IMAGE} AS web-none
RUN mkdir /webout

FROM web-${WEB} AS web

# ---- server -----------------------------------------------------------------
FROM ${RUST_IMAGE} AS build
ARG PHOTOVAULT_COMMIT=""
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY server server
COPY core core
ENV CARGO_TERM_COLOR=never PHOTOVAULT_COMMIT=${PHOTOVAULT_COMMIT}
RUN --mount=type=secret,id=ca,required=false \
    --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    if [ -s /run/secrets/ca ]; then cat /run/secrets/ca >> /etc/ssl/certs/ca-certificates.crt; fi && \
    cargo build --release --locked -p photovault-server --bin photovault && \
    cp target/release/photovault /photovault && \
    mkdir -p /rootfs/data /rootfs/config /rootfs/app/web

FROM ${RUNTIME_IMAGE}
LABEL org.opencontainers.image.title="photovault" \
      org.opencontainers.image.description="End-to-end encrypted photo server compatible with the Ente Photos apps. Based on Ente (ente.com)." \
      org.opencontainers.image.licenses="AGPL-3.0-only" \
      org.opencontainers.image.source="https://github.com/Krifmaz/photovault"
COPY --from=build /photovault /usr/local/bin/photovault
COPY --from=build --chown=65532:65532 /rootfs/ /
COPY --from=web --chown=65532:65532 /webout/ /app/web/
ENV PHOTOVAULT_LISTEN=0.0.0.0:8080 \
    PHOTOVAULT_STORAGE_PATH=/data \
    PHOTOVAULT_CONFIG_DIR=/config \
    PHOTOVAULT_WEB_DIR=/app/web \
    PHOTOVAULT_HEALTHCHECK_URL=http://127.0.0.1:8080/healthz
USER 65532:65532
EXPOSE 8080
VOLUME ["/data", "/config"]
HEALTHCHECK --interval=30s --timeout=5s --start-period=30s --retries=3 \
    CMD ["/usr/local/bin/photovault", "healthcheck"]
ENTRYPOINT ["/usr/local/bin/photovault"]
CMD ["serve"]
