# Multi-stage build homesign serveru.
# Cíl je Linux, kde se macOS-only `aos_kit` z omnisette vůbec nekompiluje.

FROM rust:1-bookworm AS build
WORKDIR /src

# Systémové závislosti pro build (openssl/pkg-config kvůli tranzitivním crateům).
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config libssl-dev cmake clang \
    && rm -rf /var/lib/apt/lists/*

# Nejdřív manifesty kvůli cache vrstvě závislostí.
COPY Cargo.toml Cargo.lock ./
COPY server/Cargo.toml server/Cargo.toml
COPY cli/Cargo.toml cli/Cargo.toml
RUN mkdir -p server/src cli/src \
    && echo 'fn main(){}' > server/src/main.rs \
    && echo 'fn main(){}' > cli/src/main.rs \
    && cargo build --release -p homesign-server || true

# Teď skutečné zdroje.
COPY server/ server/
COPY cli/ cli/
RUN touch server/src/main.rs \
    && cargo build --release -p homesign-server

FROM debian:bookworm-slim AS runtime
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=build /src/target/release/homesign-server /usr/local/bin/homesign-server

ENV HOMESIGN_BIND=0.0.0.0:8080
ENV HOMESIGN_DATA=/data
ENV ANISETTE_URL=http://anisette:6969
VOLUME ["/data"]
EXPOSE 8080

CMD ["homesign-server"]
