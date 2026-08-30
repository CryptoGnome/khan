# --- build stage ---
FROM rust:1-slim-bookworm AS build
WORKDIR /src
# Cache deps first
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo "fn main(){}" > src/main.rs && cargo build --release || true
# Real sources
COPY src ./src
RUN touch src/main.rs && cargo build --release

# --- runtime stage ---
FROM debian:bookworm-slim
# git (local version control), python3 + pip/venv and nodejs + npm (custom
# tools, and the SDKs agents install for themselves), ca-certificates (TLS).
# Both runtimes are baked in because only /data survives a restart: an agent
# that apt-installs a toolchain loses it on the next deploy and pays for the
# install again.
RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates git python3 python3-pip python3-venv nodejs npm \
    && rm -rf /var/lib/apt/lists/*
# Debian marks its python "externally managed" (PEP 668), which blocks plain
# `pip install`. The container is the sandbox, so let agents install freely.
ENV PIP_BREAK_SYSTEM_PACKAGES=1
# Send pip installs to the volume instead of the container filesystem, which is
# thrown away on every deploy. Without this an agent reinstalls its libraries
# after each restart, and pays for the round trip that discovers they are gone.
# PYTHONUSERBASE is read by python itself, so the packages are importable with
# no PYTHONPATH juggling and no change to how agents call pip.
# Caveat: `pip install` inside a virtualenv refuses --user; there, pass
# PIP_USER=0 (the venv already keeps its own packages).
ENV PYTHONUSERBASE=/data/.python
ENV PIP_USER=1

# Playwright + Chromium baked into the image: the browser rung of the fetch
# ladder (JS-heavy pages, soft anti-bot walls) and rendered-page QA. Installed
# system-wide (not on the volume) so agents never reinstall it after a deploy —
# losing it cost a worker ten minutes of apt/pip archaeology per restart.
ENV PLAYWRIGHT_BROWSERS_PATH=/opt/ms-playwright
RUN PIP_USER=0 pip3 install playwright \
    && playwright install --with-deps chromium \
    && rm -rf /var/lib/apt/lists/*

COPY --from=build /src/target/release/khan /usr/local/bin/khan
# Baked default config; override by putting khan.toml on the volume and setting KHAN_CONFIG=/data/khan.toml.
COPY khan.toml.example /app/khan.toml
COPY skills /app/skills
ENV KHAN_CONFIG=/app/khan.toml

# /data is a Railway volume: khan.db and workspace/ live here so they persist across redeploys.
WORKDIR /data
# Live log viewer (overridden by Railway's PORT variable when networking is enabled).
EXPOSE 8080
ENTRYPOINT ["khan"]
# `auto` resumes an existing mission, or starts from KHAN_DIRECTIVE on first boot.
CMD ["auto"]
