FROM rust:1.99.1-bookworm AS build
WORKDIR /app
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates ./crates
COPY migrations ./migrations
RUN cargo build --release --locked -p porch-server

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/*     && useradd --system --uid 10001 porch
COPY --from=build /app/target/release/porch-server /usr/local/bin/porch-server
USER 10001:10001
ENV PORCH_BIND=0.0.0.0:38147
EXPOSE 38147
STOPSIGNAL SIGINT
ENTRYPOINT ["porch-server"]
