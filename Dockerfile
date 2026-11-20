# syntax=docker/dockerfile:1
# Stage 1: build the WASM engine (wasm32 + wasm-bindgen + wasm-opt) and the server.
FROM rust:1-slim-bookworm AS build
ARG WASM_BINDGEN_VERSION=0.2.130
ARG BINARYEN_VERSION=version_123
ENV CARGO_BUILD_JOBS=2 CARGO_NET_RETRY=5
RUN apt-get update && apt-get install -y --no-install-recommends build-essential ca-certificates curl && rm -rf /var/lib/apt/lists/*
# distro binaryen is too old for current rustc output; use an upstream release
RUN curl -fsSL https://github.com/WebAssembly/binaryen/releases/download/${BINARYEN_VERSION}/binaryen-${BINARYEN_VERSION}-x86_64-linux.tar.gz | tar -xz -C /opt \
 && ln -s /opt/binaryen-${BINARYEN_VERSION}/bin/wasm-opt /usr/local/bin/wasm-opt && wasm-opt --version
RUN rustup target add wasm32-unknown-unknown \
 && nice -n 19 cargo install wasm-bindgen-cli --version ${WASM_BINDGEN_VERSION} --locked
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY proto ./proto
COPY engine ./engine
COPY server ./server
COPY web ./web
COPY scripts ./scripts
COPY tests/fixtures ./tests/fixtures
# fails the build when engine_bg.wasm is larger than 2 MiB
RUN nice -n 19 bash scripts/build-web.sh
RUN nice -n 19 cargo build --locked --profile server -p draw-server

# Stage 2: minimal runtime, non-root.
FROM debian:bookworm-slim
RUN useradd --system --uid 10001 --no-create-home --shell /usr/sbin/nologin draw \
 && mkdir -p /data /app/static && chown draw:draw /data
COPY --from=build /src/target/server/draw-server /app/draw-server
COPY --from=build /src/dist /app/static
USER 10001
ENV DRAW_ADDR=0.0.0.0:8080 DRAW_DB=/data/draw.db DRAW_STATIC=/app/static
EXPOSE 8080
VOLUME ["/data"]
ENTRYPOINT ["/app/draw-server"]
