# syntax=docker/dockerfile:1.7

FROM node:22-bookworm-slim AS css
WORKDIR /app

RUN npm install -g pnpm@10

COPY package.json pnpm-lock.yaml ./
RUN pnpm install --frozen-lockfile

COPY backend/src/http/ui ./backend/src/http/ui
RUN pnpm run build:css

FROM rust:1-bookworm AS builder
WORKDIR /app

COPY backend/Cargo.toml backend/Cargo.lock ./backend/
COPY backend/src ./backend/src
COPY --from=css /app/backend/src/http/ui/styles.generated.css ./backend/src/http/ui/styles.generated.css

WORKDIR /app/backend
RUN cargo build --release --locked

FROM debian:bookworm-slim AS runtime

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates ffmpeg \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --gid 10001 app \
    && useradd --create-home --uid 10001 --gid 10001 app \
    && mkdir -p /data/uploads \
    && chown -R app:app /data

COPY --from=builder /app/backend/target/release/video-analysis /usr/local/bin/video-analysis

ENV PORT=8080 \
    SURREAL_URL=surrealkv:///data/surrealdb \
    SURREAL_NAMESPACE=video_analysis \
    SURREAL_DATABASE=app \
    SURREAL_UPLOAD_BUCKET_PATH=/data/uploads

VOLUME ["/data"]
EXPOSE 8080

USER app
CMD ["/usr/local/bin/video-analysis"]
