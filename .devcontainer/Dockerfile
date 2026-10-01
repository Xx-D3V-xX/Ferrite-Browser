FROM ubuntu:22.04

ENV DEBIAN_FRONTEND=noninteractive
ENV CARGO_HOME=/usr/local/cargo
ENV RUSTUP_HOME=/usr/local/rustup
ENV PATH=/usr/local/cargo/bin:$PATH

# ── System dependencies ────────────────────────────────────────────────────
# Servo requires: clang, lld, cmake, pkg-config, python3, gstreamer,
# libdbus, libfreetype, libfontconfig, libssl, libxcb + friends
RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential \
    curl \
    git \
    pkg-config \
    cmake \
    clang \
    lld \
    llvm \
    python3 \
    python3-pip \
    libssl-dev \
    libdbus-1-dev \
    libfreetype6-dev \
    libfontconfig1-dev \
    libglib2.0-dev \
    libgstreamer1.0-dev \
    libgstreamer-plugins-base1.0-dev \
    libgstreamer-plugins-bad1.0-dev \
    gstreamer1.0-plugins-base \
    gstreamer1.0-plugins-good \
    gstreamer1.0-plugins-bad \
    libxcb1-dev \
    libxcb-render0-dev \
    libxcb-shape0-dev \
    libxcb-xfixes0-dev \
    libx11-dev \
    libxext-dev \
    libxrandr-dev \
    libxi-dev \
    libxcursor-dev \
    sqlite3 \
    libsqlite3-dev \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# ── Rust toolchain ─────────────────────────────────────────────────────────
# No wasm32-unknown-unknown target: that was for the Wasm/Extism extension
# sandbox, deleted per docs/REBUILD_DIRECTIVE.md §4 — nothing in the current
# workspace targets Wasm.
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | \
    sh -s -- -y --default-toolchain stable --no-modify-path && \
    rustup component add clippy rustfmt rust-analyzer

# ── Pre-warm Cargo registry ────────────────────────────────────────────────
# Copy only manifests first so this layer is cached unless deps change.
# Workspace lives at the repo root since the P0 flatten (was Browser/*).
# Members list here must match Cargo.toml's [workspace] members exactly —
# ferrite-capability-broker/ferrite-policy/ferrite-sandbox are deleted, not
# stubbed, do not add them back here.
WORKDIR /workspace
COPY Cargo.toml ./Cargo.toml
COPY crates/ferrite-shell/Cargo.toml     ./crates/ferrite-shell/Cargo.toml
COPY crates/ferrite-servo/Cargo.toml     ./crates/ferrite-servo/Cargo.toml
COPY crates/ferrite-ui/Cargo.toml        ./crates/ferrite-ui/Cargo.toml
COPY crates/ferrite-audit-log/Cargo.toml ./crates/ferrite-audit-log/Cargo.toml
COPY crates/ferrite-agent/Cargo.toml     ./crates/ferrite-agent/Cargo.toml
COPY crates/ferrite-ipi/Cargo.toml       ./crates/ferrite-ipi/Cargo.toml
COPY crates/ferrite-eval/Cargo.toml      ./crates/ferrite-eval/Cargo.toml

# Create stub lib.rs / main.rs for each crate so `cargo fetch` resolves fully
RUN for dir in \
        ferrite-servo \
        ferrite-ui \
        ferrite-audit-log \
        ferrite-agent \
        ferrite-ipi \
        ferrite-eval; do \
    mkdir -p crates/$dir/src && \
    echo "// stub" > crates/$dir/src/lib.rs; \
    done && \
    mkdir -p crates/ferrite-shell/src && \
    echo "fn main(){}" > crates/ferrite-shell/src/main.rs

# Fetch all dependencies (populates CARGO_HOME registry cache)
RUN cargo fetch

# ── Working directory for actual development ───────────────────────────────
WORKDIR /workspace
