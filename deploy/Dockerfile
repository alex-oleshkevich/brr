# syntax=docker/dockerfile:1
FROM oven/bun:1 AS ui
WORKDIR /src
COPY web web
COPY extension extension
RUN bun install --cwd web --frozen-lockfile && bun run --cwd web build
RUN bun run --cwd extension build.ts

FROM rust:1.96-trixie AS build
RUN apt-get update && apt-get install -y --no-install-recommends just && rm -rf /var/lib/apt/lists/*
WORKDIR /src
COPY Cargo.toml Cargo.lock justfile ./
COPY .cargo .cargo
COPY crates crates
COPY --from=ui /src/web/build web/build
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    just build-release && install -D target/release/brr /out/brr

FROM debian:trixie-slim
RUN apt-get update && apt-get install -y --no-install-recommends \
        tini xvfb xauth ca-certificates chromium fonts-liberation fonts-noto-color-emoji \
        libgtk-3-0t64 libdbus-glib-1-2 libxt6t64 libasound2t64 libx11-xcb1 libpci3 \
    && rm -rf /var/lib/apt/lists/*
RUN useradd --uid 1000 --create-home --shell /usr/sbin/nologin brr \
    && mkdir -p /data /run/brr \
    && chown brr:brr /data /run/brr \
    && chmod 0700 /run/brr
COPY --from=build /out/brr /usr/local/bin/brr
COPY --from=ui /src/extension/dist /usr/share/brr/extension
# MPL-2.0 source offer for autoconsent, and Playwright's license for the injected script
COPY crates/brr-content/vendor/autoconsent/ /usr/share/doc/brr/autoconsent/
COPY deploy/NOTICE /usr/share/doc/brr/NOTICE
COPY crates/brr-engine/assets/playwright-LICENSE crates/brr-engine/assets/playwright-NOTICE /usr/share/doc/brr/playwright/
USER brr
ENV XDG_CONFIG_HOME=/data/config \
    XDG_DATA_HOME=/data/data \
    XDG_STATE_HOME=/data/state \
    XDG_RUNTIME_DIR=/run/brr \
    BRR_SERVER__BIND=0.0.0.0:9898 \
    BRR_VAULT__KEY_SOURCE=file
VOLUME /data
EXPOSE 9898
ENTRYPOINT ["tini", "--", "brr"]
CMD ["daemon", "run"]
