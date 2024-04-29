ARG UBI_MINIMAL_BASE_IMAGE=registry.access.redhat.com/ubi9/ubi-minimal
ARG UBI_BASE_IMAGE_TAG=latest
ARG PROTOC_VERSION=26.0

## Rust builder ################################################################
# Specific debian version so that compatible glibc version is used
FROM rust:1.77-bullseye as rust-builder
ARG PROTOC_VERSION

ENV CARGO_REGISTRIES_CRATES_IO_PROTOCOL=sparse

# Install protoc, no longer included in prost crate
RUN cd /tmp && \
    curl -L -O https://github.com/protocolbuffers/protobuf/releases/download/v${PROTOC_VERSION}/protoc-${PROTOC_VERSION}-linux-x86_64.zip && \
    unzip protoc-*.zip -d /usr/local && rm protoc-*.zip

WORKDIR /app

COPY rust-toolchain.toml rust-toolchain.toml

RUN rustup component add rustfmt

## Orchestrator builder #########################################################
FROM rust-builder as fms-orchestr8-builder

COPY build.rs *.toml LICENSE /app
COPY config/ /app/config
COPY protos/ /app/protos
COPY src/ /app/src

WORKDIR /app

# TODO: Make releases via cargo-release
RUN cargo install --root /app/ --path .


## Release Image ################################################################

FROM ${UBI_MINIMAL_BASE_IMAGE}:${UBI_BASE_IMAGE_TAG}

COPY --from=fms-orchestr8-builder /app/bin/ /app/bin/
COPY config /app/config

RUN microdnf install -y --disableplugin=subscription-manager shadow-utils && \
    microdnf clean all --disableplugin=subscription-manager


ENV ORCHESTRATOR_CONFIG /app/config/config.yaml

CMD /app/bin/fms-orchestr8