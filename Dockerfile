ARG RUST_VERSION=1.41
ARG DEBIAN_CODE_NAME=buster

FROM rust:${RUST_VERSION}-slim-${DEBIAN_CODE_NAME} AS builder
WORKDIR /workdir
COPY ./ ./
RUN cargo build --release --example dump

FROM debian:${DEBIAN_CODE_NAME}-slim AS runtime
WORKDIR /app
COPY --from=builder /workdir/target/release/examples/dump /app/dump

RUN groupadd rust && \
  useradd -g rust rust && \
  chown rust:rust /app

USER rust

ENTRYPOINT ["./dump"]
