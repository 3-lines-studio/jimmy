# syntax=docker/dockerfile:1
FROM rust:1.98-bookworm AS builder
RUN rustup toolchain install nightly --profile minimal && rustup default nightly
WORKDIR /build

COPY Cargo.toml Cargo.lock /build/jimmy/
COPY src /build/jimmy/src
WORKDIR /build/jimmy
RUN cargo build --release --locked

FROM debian:bookworm-slim
ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates libcurl4 bash git curl less unzip xz-utils bzip2 \
        build-essential chromium \
    && rm -rf /var/lib/apt/lists/*

ARG WAX_VERSION=v0.3.2
RUN arch="$(dpkg --print-architecture)" \
    && case "$arch" in \
         amd64) asset=wax-linux-x86_64 ;; \
         arm64) asset=wax-linux-aarch64 ;; \
         *) echo "unsupported architecture: $arch" >&2; exit 1 ;; \
       esac \
    && base="https://github.com/3-lines-studio/wax/releases/download/${WAX_VERSION}" \
    && curl -fsSL -o "/tmp/$asset" "$base/$asset" \
    && curl -fsSL -o /tmp/SHA256SUMS "$base/SHA256SUMS" \
    && (cd /tmp && grep " $asset$" SHA256SUMS | sha256sum -c -) \
    && install -m 0755 "/tmp/$asset" /usr/local/bin/wax \
    && rm -f "/tmp/$asset" /tmp/SHA256SUMS

ENV RUSTUP_HOME=/root/.rustup
ENV CARGO_HOME=/root/.cargo
RUN curl -fsSL https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain nightly -c rustfmt -c clippy

ENV MISE_DATA_DIR=/root/.local/share/mise
ENV MISE_CONFIG_DIR=/root/.config/mise
ENV MISE_YES=1
ENV PATH=/root/.cargo/bin:/root/.local/share/mise/shims:/root/.local/bin:$PATH
RUN curl -fsSL https://mise.run | sh
COPY mise.toml /root/.config/mise/config.toml
RUN mise install

RUN git config --system user.name "Jimmy" \
    && git config --system user.email "jimmy@3lines.studio" \
    && git config --system credential."https://github.com".helper '!gh auth git-credential' \
    && git config --system advice.detachedHead false
ENV GIT_TERMINAL_PROMPT=0

COPY --from=builder /build/jimmy/target/release/jimmy /usr/local/bin/jimmy

ENV XDG_CONFIG_HOME=/root/.config
COPY SYSTEM.md /root/.config/axe/SYSTEM.md
COPY --chmod=0755 bin/search /usr/local/bin/search
COPY --chmod=0755 bin/history /usr/local/bin/history
ENV WAX_NO_SANDBOX=1

WORKDIR /data
CMD ["jimmy"]
