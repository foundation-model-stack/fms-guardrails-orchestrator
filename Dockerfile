ARG UBI_MINIMAL_BASE_IMAGE=registry.access.redhat.com/ubi9/ubi-minimal
ARG UBI_BASE_IMAGE_TAG=latest
ARG PROTOC_VERSION=26.0
ARG CONFIG_FILE=config/config.yaml

## Rust builder ################################################################
# Specific debian version so that compatible glibc version is used
FROM rust:1.80.1-bullseye as rust-builder
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
FROM rust-builder as fms-guardrails-orchestr8-builder

COPY build.rs *.toml LICENSE /app/
COPY ${CONFIG_FILE} /app/config/config.yaml
COPY protos/ /app/protos/
COPY src/ /app/src/

WORKDIR /app

# TODO: Make releases via cargo-release
RUN cargo install --root /app/ --path .

## Tests stage ##################################################################
FROM fms-guardrails-orchestr8-builder as tests
RUN cargo test

## Lint stage ###################################################################
FROM fms-guardrails-orchestr8-builder as lint
RUN cargo clippy --all-targets --all-features -- -D warnings

## Formatting check stage #######################################################
FROM fms-guardrails-orchestr8-builder as format
RUN cargo fmt --check

## Release Image ################################################################

FROM ${UBI_MINIMAL_BASE_IMAGE}:${UBI_BASE_IMAGE_TAG} as fms-guardrails-orchestr8-release

COPY --from=fms-guardrails-orchestr8-builder /app/bin/ /app/bin/
COPY ${CONFIG_FILE} /app/config/config.yaml

RUN microdnf install -y --disableplugin=subscription-manager shadow-utils compat-openssl11 && \
    microdnf clean all --disableplugin=subscription-manager

RUN groupadd --system orchestr8 --gid 1001 && \
    adduser --system --uid 1001 --gid 0 --groups orchestr8 \
    --create-home --home-dir /app --shell /sbin/nologin \
    --comment "FMS Orchestrator User" orchestr8

USER orchestr8

ENV ORCHESTRATOR_CONFIG /app/config/config.yaml

CMD /app/bin/fms-guardrails-orchestr8
