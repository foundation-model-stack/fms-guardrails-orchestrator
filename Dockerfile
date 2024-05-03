## Global Args #################################################################
ARG BASE_UBI_IMAGE_TAG=9.3
ARG PROTOC_VERSION=26.0

## Base ########################################################################
FROM --platform=linux/amd64 registry.access.redhat.com/ubi9/ubi:${BASE_UBI_IMAGE_TAG} as base

RUN dnf remove -y --disableplugin=subscription-manager \
        subscription-manager \
    && dnf install -y make compat-openssl11 \
        procps \
    && dnf clean all

ENV LANG=C.UTF-8 \
    LC_ALL=C.UTF-8

## Build #######################################################################
FROM rust:1-bullseye as build
ARG PROTOC_VERSION

COPY build.rs Cargo.toml LICENSE /app/
COPY protos/ /app/protos
COPY src/ /app/src

# Install protoc
RUN cd /tmp && \
    curl -L -O https://github.com/protocolbuffers/protobuf/releases/download/v${PROTOC_VERSION}/protoc-${PROTOC_VERSION}-linux-x86_64.zip && \
    unzip protoc-*.zip -d /usr/local && rm protoc-*.zip

WORKDIR /app

RUN cargo install --root /app/ --path .

## Release #####################################################################
FROM base

COPY config/ /app/config

RUN useradd -u 2000 orchestr8 -m -g 0

ENV HOME=/home/orchestr8 \
    ORCHESTRATOR_CONFIG=/app/config/config.yaml

RUN chmod -R g+rwx ${HOME}

COPY --from=build /app/bin/fms-orchestr8 /app/bin/fms-orchestr8
COPY config /config

# Run as non-root user by default
USER orchestr8

CMD /app/bin/fms-orchestr8