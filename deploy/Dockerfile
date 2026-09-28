FROM rust:1.85 as builder

WORKDIR /app
# workspace 的成员清单位于根 Cargo.toml，必须把全部成员目录一起复制进来，
# 否则 cargo 会因为找不到 gui/ 成员而直接报错。
COPY . .
# 只构建服务端，GUI 不会参与编译
RUN cargo build --release -p secunzip-server

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*

WORKDIR /app
# workspace 的产物都在根目录的 target/ 下
COPY --from=builder /app/target/release/secunzip-server .

ENV DATABASE_URL=sqlite:/data/secunzip.db?mode=rwc
ENV SECUNZIP_PORT=8090
EXPOSE 8090

VOLUME ["/data"]

CMD ["./secunzip-server"]
