# syntax=docker/dockerfile:1.7
#
# Release image, used only by .github/workflows/release.yml. It packages the
# server binaries that workflow already built (in Debian bookworm, matching
# distroless glibc) so the arm64 image needs no emulated Rust build.
# Keep the runtime settings in step with the root Dockerfile.
FROM gcr.io/distroless/cc-debian12:nonroot
ARG TARGETARCH
LABEL org.opencontainers.image.title="photovault" \
      org.opencontainers.image.description="End-to-end encrypted photo server compatible with the Ente Photos apps. Based on Ente (ente.com)." \
      org.opencontainers.image.licenses="AGPL-3.0-only" \
      org.opencontainers.image.source="https://github.com/Krifmaz/photovault"
COPY --chmod=0755 bin/${TARGETARCH}/photovault /usr/local/bin/photovault
COPY --chown=65532:65532 rootfs/ /
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
