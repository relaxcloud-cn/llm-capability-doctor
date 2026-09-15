FROM debian:bookworm-slim

# Only the OS trust store is installed; no OMP, Node.js, Bun, or source checkout.
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /work
