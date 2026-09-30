# syntax=docker/dockerfile:1
FROM rust:1.98-bookworm AS builder
RUN rustup toolchain install nightly --profile minimal && rustup default nightly
WORKDIR /build

COPY Cargo.toml Cargo.lock /build/jimmy/
COPY src /build/jimmy/src
COPY web /build/jimmy/web
WORKDIR /build/jimmy
RUN cargo build --release --locked

FROM debian:bookworm-slim
ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates libcurl4 bash git curl less unzip xz-utils bzip2 gzip \
        build-essential chromium tini ffmpeg \
    && rm -rf /var/lib/apt/lists/*

ARG BQX_VERSION=v0.3.2
ARG PGX_VERSION=v0.2.1
RUN curl -fsSL https://ax.3lines.studio/install.sh -o /tmp/ax-install.sh \
    && AX_PREFIX=/usr/local VERSION="$BQX_VERSION" sh /tmp/ax-install.sh bqx \
    && AX_PREFIX=/usr/local VERSION="$PGX_VERSION" sh /tmp/ax-install.sh pgx \
    && rm /tmp/ax-install.sh

ENV CARGO_TARGET_DIR=/tmp/cargo-target

ENV MISE_DATA_DIR=/root/.local/share/mise
ENV MISE_CONFIG_DIR=/root/.config/mise
ENV MISE_YES=1
ENV PATH=/root/.local/share/mise/shims:/root/.cargo/bin:/root/.local/bin:$PATH
RUN curl -fsSL https://mise.run | sh
COPY mise.toml /root/.config/mise/config.toml
RUN mise install

ENV MBX_GC_MAX_SIZE=10GiB
ENV MBX_GC_INCREMENTAL_MAX_SIZE=10GiB

RUN export PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD=1 VENV=/opt/browse-venv \
    && uv venv --clear "$VENV" \
    && uv pip install --python "$VENV/bin/python" playwright \
    && "$VENV/bin/playwright" install ffmpeg

ENV UV_PYTHON_INSTALL_DIR=/data/uv/python
ENV UV_CACHE_DIR=/data/uv/cache
ENV UV_PYTHON_PREFERENCE=only-managed

RUN git config --system user.name "Jimmy" \
    && git config --system user.email "jimmy@3lines.studio" \
    && git config --system credential."https://github.com".helper '!gh auth git-credential' \
    && git config --system advice.detachedHead false
ENV GIT_TERMINAL_PROMPT=0

COPY --from=builder /build/jimmy/target/release/jimmy /usr/local/bin/jimmy
RUN ln -sf /usr/local/bin/jimmy /usr/local/bin/heimdall \
    && ln -sf /usr/local/bin/jimmy /usr/local/bin/doppler \
    && heimdall help > /dev/null \
    && doppler help > /dev/null

ENV XDG_CONFIG_HOME=/root/.config
COPY prompts /usr/local/share/jimmy/prompts
COPY skills /usr/local/share/jimmy/skills
COPY --chmod=0755 bin/recall /usr/local/bin/recall
COPY --chmod=0755 bin/browse /usr/local/bin/browse
COPY --chmod=0755 bin/stats /usr/local/bin/stats
COPY --chmod=0755 bin/gen-image /usr/local/bin/gen-image
WORKDIR /data
ENTRYPOINT ["/usr/bin/tini", "--"]
CMD ["jimmy"]
