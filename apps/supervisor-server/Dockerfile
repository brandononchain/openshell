FROM rust:1.82 as builder
WORKDIR /app
COPY . .
RUN cargo build --release --manifest-path apps/supervisor-server/Cargo.toml

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=builder /app/apps/supervisor-server/target/release/supervisor-server /usr/local/bin/supervisor-server
ENV SUPERVISOR_DATA_DIR=/data
EXPOSE 8080
VOLUME ["/data"]
CMD ["/usr/local/bin/supervisor-server"]
