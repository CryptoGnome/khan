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

COPY --from=build /src/target/release/khan /usr/local/bin/khan
# Baked default config; override by putting khan.toml on the volume and setting KHAN_CONFIG=/data/khan.toml.
COPY khan.toml.example /app/khan.toml
ENV KHAN_CONFIG=/app/khan.toml

# /data is a Railway volume: khan.db and workspace/ live here so they persist across redeploys.
WORKDIR /data
# Live log viewer (overridden by Railway's PORT variable when networking is enabled).
EXPOSE 8080
ENTRYPOINT ["khan"]
# `auto` resumes an existing mission, or starts from KHAN_DIRECTIVE on first boot.
CMD ["auto"]
