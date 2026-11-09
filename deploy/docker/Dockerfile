# Static diffyman-server image for parity checks (docs/operations.md).
#
# Build from the repository root. scripts/build-server-image.sh wraps this
# with a temporary builder, so its build cache is removed afterwards:
#   docker build -f deploy/docker/Dockerfile -t diffyman-server:local .
# For s3:// stores add --build-arg FEATURES=s3 (builds aws-lc-rs from C).
#
# The binary is linked statically against musl on Alpine and runs on
# distroless/static as the `nonroot` user (UID 65532): no shell, no package
# manager, no libc. Base images are pinned by digest; when updating one,
# update its tag and digest together.

FROM rust:1.99.1-alpine3.24@sha256:44819b27be4c97b0e4d7203bd2e02c4820f97700211b6a37fb4c246a2de59383 AS build
ARG FEATURES=
RUN apk add --no-cache musl-dev \
 && if [ -n "$FEATURES" ]; then apk add --no-cache cmake make perl linux-headers; fi
WORKDIR /src
# The build context is an allowlist (Dockerfile.dockerignore): manifests,
# Cargo.lock, crate sources and the offline query data in .sqlx/. The
# toolchain file is left out on purpose: the image's Rust is the same release
# without the extra components the file asks for.
COPY . .
# musl targets link statically by default (crt-static); setting it in
# RUSTFLAGS would also hit proc-macros, which musl cannot build statically.
ENV SQLX_OFFLINE=true \
    CARGO_TERM_COLOR=never \
    CARGO_NET_RETRY=5
RUN cargo build --release --locked -p diffyman-server --bin diffyman-server ${FEATURES:+--features "$FEATURES"} \
 && install -D -m 0755 target/release/diffyman-server /out/diffyman-server \
 && mkdir -p /out/data/blobs /out/data/tmp \
 && chmod 0700 /out/data /out/data/blobs /out/data/tmp \
 && chown -R 65532:65532 /out/data

FROM gcr.io/distroless/static-debian13:nonroot@sha256:25ac8f0c3b2334a9eb05452266bd4697864a8c8ef0ef71e5e6480d7f92868329
LABEL org.opencontainers.image.title="diffyman-server" \
      org.opencontainers.image.description="Local REST service for the diffyman diff engine" \
      org.opencontainers.image.source="https://github.com/reddy-sh/diffyman" \
      org.opencontainers.image.url="https://reddy.sh" \
      org.opencontainers.image.licenses="MIT OR Apache-2.0"
COPY --from=build /out/diffyman-server /usr/local/bin/diffyman-server
# /data holds blobs, temporary files and the admin token. A named volume
# mounted here starts with this folder's owner (nonroot) and mode (0700).
COPY --from=build --chown=65532:65532 /out/data /data
USER 65532:65532
# A container is a deployment: `prod` never reads .env.local and requires
# DIFFYMAN_DB_URL and DIFFYMAN_ALLOWED_HOSTS. It listens on all interfaces
# inside the container; publish the port on 127.0.0.1 only (compose.yaml).
ENV DIFFYMAN_ENV=prod \
    DIFFYMAN_BIND=0.0.0.0:8080 \
    DIFFYMAN_STORE_URL=file:///data/blobs \
    DIFFYMAN_TMP_DIR=/data/tmp \
    DIFFYMAN_ADMIN_TOKEN_FILE=/data/admin-token
EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/diffyman-server"]
CMD ["serve"]
