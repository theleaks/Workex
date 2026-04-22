# V8 build + kenton-response benchmarks, reproducible.
#
# Build: docker build -t kenton-bench .
# Run:   docker run --rm -v "$(pwd)/results:/work/results" kenton-bench
#
# First build takes 30-90 min and ~20 GB disk inside the image.
# Subsequent runs reuse the built V8 via layer cache.
#
# Two-stage build:
#   Stage 1 (v8-base): Ubuntu + V8 monolith. Slow (~30 min), rarely re-runs.
#   Stage 2 (kenton-bench): adds our benchmarks. Fast (~1 min), iterated on.
# If Docker's buildkit cache evicts stage 1, you can pin it:
#   docker build --target v8-base -t kenton-v8-base .
# and then `FROM kenton-v8-base` in subsequent edits.

FROM ubuntu:24.04 AS v8-base

# V8 12.8 (stable, builds cleanly with system gcc + libstdc++).
# workerd currently pins V8 14.7.173.16; we document the version gap in
# the PR description. API between 12.8 and 14.7 for ValueSerializer /
# Isolate / HeapStatistics is source-compatible (no breaking changes in
# our usage); relative performance differences are expected to be <20%.
ARG V8_REV=branch-heads/12.8
ENV DEBIAN_FRONTEND=noninteractive

RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential \
    clang \
    cmake \
    ninja-build \
    pkg-config \
    git \
    curl \
    ca-certificates \
    python3 \
    python3-pip \
    lsb-release \
    file \
    sudo \
    xz-utils \
    libglib2.0-dev \
 && rm -rf /var/lib/apt/lists/*

WORKDIR /work

# depot_tools
RUN git clone --depth=1 https://chromium.googlesource.com/chromium/tools/depot_tools.git /opt/depot_tools
ENV PATH=/opt/depot_tools:$PATH
ENV GCLIENT_SUPPRESS_GIT_VERSION_WARNING=1

# Force depot_tools to bootstrap its Python/git wrappers (creates
# python3_bin_reldir.txt, downloads cipd client, etc.). Without this,
# `gn gen` fails with "python3_bin_reldir.txt not found".
RUN gclient --version || true

# Fetch V8 + check out the target rev + gclient-sync deps, all in a
# single RUN so cipd's internal renames stay within one overlay layer
# (otherwise we get "invalid cross-device link" on overlay2).
RUN mkdir -p /work/vendor \
 && cd /work/vendor \
 && fetch --nohooks v8 \
 && cd v8 \
 && git fetch --depth=1 origin ${V8_REV} \
 && git checkout FETCH_HEAD \
 && gclient sync -D --no-history

# Skip V8's install-build-deps.sh — it pulls Chromium desktop deps
# (X, fonts, audio) we don't need for v8_monolith. Our apt-get above
# plus use_sysroot=false below cover what we actually need.

# Configure and build v8_monolith.
RUN cd /work/vendor/v8 && \
    gn gen out/x64.release --args='is_debug=false target_cpu="x64" \
        v8_monolithic=true v8_use_external_startup_data=false \
        use_custom_libcxx=false is_component_build=false \
        v8_enable_sandbox=true treat_warnings_as_errors=false \
        use_sysroot=false' && \
    ninja -C out/x64.release v8_monolith

# =====================================================================
# Stage 2: add our benchmarks on top of v8-base. Iterations happen here.
# =====================================================================
FROM v8-base AS kenton-bench

# Copy our project sources and build the benchmarks.
COPY benchmarks /work/benchmarks
COPY src /work/src
COPY scripts /work/scripts
COPY build_all.sh run_all.sh /work/
RUN chmod +x /work/build_all.sh /work/run_all.sh /work/scripts/*.py 2>/dev/null || true

# Build P1/P2 and v8-cow tools against the already-built V8.
# With is_clang=false + use_custom_libcxx=false + sandbox=false, V8 uses
# system gcc and libstdc++, so benchmarks can link normally with g++.
ENV V8_DIR=/work/vendor/v8
RUN cd /work/benchmarks && ./build.sh
RUN cd /work/src/v8-cow && ./build.sh

# Default: run every benchmark and dump JSON into /work/results.
CMD ["/work/run_all.sh"]
