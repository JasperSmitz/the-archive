FROM rust:1.96.0-slim-trixie AS build
WORKDIR /build
COPY Cargo.toml Cargo.lock build.rs ./
COPY src ./src
COPY templates ./templates
COPY static ./static
COPY migrations ./migrations
RUN cargo build --release --locked

FROM debian:trixie-slim AS runtime
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates gosu postgresql-client-17 \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --gid 10001 archive \
    && useradd --uid 10001 --gid 10001 --no-create-home --shell /usr/sbin/nologin archive
WORKDIR /app
COPY --from=build /build/target/release/the-archive /app/the-archive
COPY scripts /app/scripts
ENV APP_ENV=production LISTEN_ADDR=0.0.0.0:3000 IMAGE_STORAGE_DIR=/data/images
USER 10001:10001
EXPOSE 3000
ENTRYPOINT ["/app/scripts/container-entrypoint.sh"]
CMD ["serve"]
