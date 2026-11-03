# syntax=docker/dockerfile:1.7
#
# Photovault server image: one release binary on distroless/cc (glibc +
# libgcc only, no shell, no package manager), running as uid 65532.
#
#   docker build -t photovault .
#
# Options:
#   --build-arg RUST_IMAGE=rust:1.99-bookworm   (Docker Hub instead of Google's mirror)
#   --secret id=ca,src=/path/ca.crt   (TLS-intercepting proxy; used only while building)
#   --build-arg PHOTOVAULT_COMMIT=$(git rev-parse --short HEAD)   (shown in /photovault/info)
#
# The built web app (static export in web/apps/photos/out, made before
# `docker build`) is copied to /app/web and served on the API port.
#
# libsodium is compiled from the source bundled in libsodium-sys-stable and
# SQLite is bundled by sqlx, so the binary needs nothing beyond glibc.

ARG RUST_IMAGE=mirror.gcr.io/library/rust:1.99-bookworm
ARG RUNTIME_IMAGE=gcr.io/distroless/cc-debian12:nonroot

FROM ${RUST_IMAGE} AS build
ARG PHOTOVAULT_COMMIT=""
WORKDIR /src
COPY . .
ENV CARGO_TERM_COLOR=never PHOTOVAULT_COMMIT=${PHOTOVAULT_COMMIT}
RUN --mount=type=secret,id=ca,required=false \
    --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    if [ -s /run/secrets/ca ]; then cat /run/secrets/ca >> /etc/ssl/certs/ca-certificates.crt; fi && \
    cargo build --release --locked -p photovault-server --bin photovault && \
    cp target/release/photovault /photovault && \
    mkdir -p /rootfs/data /rootfs/config /rootfs/app/web && \
    if [ -f web/apps/photos/out/index.html ]; then cp -r web/apps/photos/out/. /rootfs/app/web/; fi && \
    for app in albums cast accounts; do if [ -f web/apps/$app/out/index.html ]; then mkdir -p /rootfs/app/web/$app && cp -r web/apps/$app/out/. /rootfs/app/web/$app/; fi; done

FROM ${RUNTIME_IMAGE}
LABEL org.opencontainers.image.title="photovault" \
      org.opencontainers.image.description="End-to-end encrypted photo server compatible with the Ente Photos apps. Based on Ente (ente.com)." \
      org.opencontainers.image.licenses="AGPL-3.0-only" \
      org.opencontainers.image.source="https://github.com/Krifmaz/photovault"
COPY --from=build /photovault /usr/local/bin/photovault
COPY --from=build --chown=65532:65532 /rootfs/ /
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
