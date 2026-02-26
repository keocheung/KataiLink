# syntax=docker/dockerfile:1.7

FROM rust:alpine AS builder
WORKDIR /app

RUN apk add --no-cache build-base musl-dev openssl-dev pkgconfig perl

COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --release \
    && cp target/release/katai_link /tmp/katai_link

FROM alpine:3.21 AS runtime
ARG TARGETARCH
ARG CODEX_VERSION=latest

RUN apk add --no-cache ca-certificates curl tar openssl

RUN set -eux; \
    case "${TARGETARCH}" in \
      amd64) codex_arch="x86_64-unknown-linux-musl" ;; \
      arm64) codex_arch="aarch64-unknown-linux-musl" ;; \
      *) echo "Unsupported TARGETARCH: ${TARGETARCH}"; exit 1 ;; \
    esac; \
    if [ "${CODEX_VERSION}" = "latest" ]; then \
      codex_url="https://github.com/openai/codex/releases/latest/download/codex-${codex_arch}.tar.gz"; \
    else \
      codex_url="https://github.com/openai/codex/releases/download/${CODEX_VERSION}/codex-${codex_arch}.tar.gz"; \
    fi; \
    curl -fsSL "${codex_url}" -o /tmp/codex.tar.gz; \
    tmp_dir="$(mktemp -d)"; \
    tar -xzf /tmp/codex.tar.gz -C "${tmp_dir}"; \
    codex_bin="$(find "${tmp_dir}" -type f -name 'codex*' | head -n 1)"; \
    test -n "${codex_bin}"; \
    cp "${codex_bin}" /usr/local/bin/codex; \
    chmod +x /usr/local/bin/codex; \
    rm -rf "${tmp_dir}" /tmp/codex.tar.gz

COPY --from=builder /tmp/katai_link /usr/local/bin/katai_link

WORKDIR /app

ENV RUST_LOG=info

ENTRYPOINT ["/usr/local/bin/katai_link"]
