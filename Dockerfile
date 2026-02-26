# syntax=docker/dockerfile:1.7

FROM rust:latest AS builder
ARG TARGETARCH
WORKDIR /app

COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN set -eux; \
    case "${TARGETARCH}" in \
      amd64) rust_target="x86_64-unknown-linux-musl" ;; \
      arm64) rust_target="aarch64-unknown-linux-musl" ;; \
      *) echo "Unsupported TARGETARCH: ${TARGETARCH}"; exit 1 ;; \
    esac; \
    rustup target add "${rust_target}"; \
    cargo build --release --target "${rust_target}"; \
    cp "target/${rust_target}/release/katai_link" /tmp/katai_link

FROM alpine:3.21 AS runtime
ARG TARGETARCH
ARG CODEX_VERSION=latest

RUN apk add --no-cache ca-certificates curl tar

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
    tar -xzf /tmp/codex.tar.gz -C /usr/local/bin codex; \
    chmod +x /usr/local/bin/codex; \
    rm -f /tmp/codex.tar.gz

COPY --from=builder /tmp/katai_link /usr/local/bin/katai_link

WORKDIR /app
COPY config.yaml /app/config.yaml

ENV RUST_LOG=info

ENTRYPOINT ["/usr/local/bin/katai_link"]
