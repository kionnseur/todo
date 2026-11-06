FROM rust:1-bookworm AS builder
WORKDIR /app
COPY Cargo.toml ./
COPY src ./src
COPY static ./static
RUN cargo build --release

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/* \
    && useradd --uid 10001 --create-home --shell /usr/sbin/nologin app \
    && mkdir -p /data && chown app:app /data
COPY --from=builder /app/target/release/todo /usr/local/bin/todo
USER app
ENV DATABASE_URL=sqlite:///data/todo.db
EXPOSE 3080
VOLUME ["/data"]
CMD ["/usr/local/bin/todo"]
