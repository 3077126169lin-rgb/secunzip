FROM rust:1.85 as builder

WORKDIR /app
COPY server/ ./server/
COPY Cargo.toml ./

WORKDIR /app/server
RUN cargo build --release

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY --from=builder /app/server/target/release/secunzip-server .

ENV DATABASE_URL=sqlite:/data/secunzip.db?mode=rwc
ENV SECUNZIP_PORT=8090
EXPOSE 8090

VOLUME ["/data"]

CMD ["./secunzip-server"]
