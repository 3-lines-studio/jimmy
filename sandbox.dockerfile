# syntax=docker/dockerfile:1
#
# El entorno del agente adentro del sandbox.
#
# Es el mismo que el del control plane (la etapa final de `Dockerfile`), sin el
# agente: el binario, los prompts, las skills y los CLIs llegan al sandbox
# publicados desde el control plane, en la versión que corresponde, y se
# instalan en estos mismos paths. Acá vive lo que no cambia con el código: el
# sistema, las toolchains que el agente usa para trabajar en los proyectos, y
# lo que necesitan sus herramientas (Chromium con su venv para `browse`).
#
# Se registra una vez por versión del entorno; el agente se actualiza sin
# reconstruirla.
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
ARG HEIMDALL_VERSION=v0.1.0
RUN curl -fsSL "https://github.com/3-lines-studio/heimdall/releases/download/${HEIMDALL_VERSION}/heimdall-linux-x64.tar.gz" \
      | tar -xz -C /usr/local/bin heimdall \
    && chmod 0755 /usr/local/bin/heimdall \
    && ln -sf /usr/local/bin/heimdall /usr/local/bin/doppler \
    && heimdall help > /dev/null \
    && doppler help > /dev/null

ENV CARGO_TARGET_DIR=/tmp/cargo-target

ENV MISE_DATA_DIR=/root/.local/share/mise
ENV MISE_CONFIG_DIR=/root/.config/mise
ENV MISE_YES=1
ENV PATH=/root/.local/share/mise/shims:/root/.cargo/bin:/root/.local/bin:/usr/local/bin:/usr/bin:/bin
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

ENV XDG_CONFIG_HOME=/root/.config
WORKDIR /work
