# Build the server with a statically linked SQLite, then run it as a non-root
# user on the small Debian image. TLS belongs to a reverse proxy in front.
#
#   docker build -t maine-sync-server apps/sync-server
#   docker run -d -p 127.0.0.1:8484:8484 -v maine-sync:/data \
#     -e MAINE_SYNC_CREATE_SECRET=change-me maine-sync-server

FROM rust:1-slim-bookworm AS build
RUN apt-get update \
    && apt-get install -y --no-install-recommends build-essential \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --release --locked

FROM debian:bookworm-slim
RUN useradd --system --create-home --home-dir /home/maine maine
COPY --from=build /src/target/release/maine-sync-server /usr/local/bin/maine-sync-server
USER maine
VOLUME /data
EXPOSE 8484
ENTRYPOINT ["maine-sync-server", "--bind", "0.0.0.0:8484", "--data-dir", "/data"]
