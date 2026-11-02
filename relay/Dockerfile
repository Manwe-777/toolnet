# iroh-relay, built from crates.io. Pinned so a rebuild is reproducible.
FROM rust:1-slim-trixie AS build
ARG IROH_RELAY_VERSION=1.4.1
RUN cargo install iroh-relay@${IROH_RELAY_VERSION} --features server --locked --root /out

FROM debian:trixie-slim
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates \
 && rm -rf /var/lib/apt/lists/*
COPY --from=build /out/bin/iroh-relay /usr/local/bin/iroh-relay
USER 65534:65534
ENTRYPOINT ["iroh-relay", "--config-path", "/config/relay.toml"]
