## Global Args #################################################################
ARG BASE_UBI_IMAGE_TAG=9.3
ARG PROTOC_VERSION=26.0

## Base Layer ##################################################################
FROM --platform=linux/amd64 registry.access.redhat.com/ubi9/ubi:${BASE_UBI_IMAGE_TAG} as base

RUN dnf remove -y --disableplugin=subscription-manager \
        subscription-manager \
    && dnf install -y make compat-openssl11 \
        procps \
    && dnf clean all

ENV LANG=C.UTF-8 \
    LC_ALL=C.UTF-8

## Build #######################################################################
FROM rust:1.77-bullseye as build
ARG PROTOC_VERSION
ARG GITHUB_TOKEN

COPY . /usr/src

# Install protoc
RUN cd /tmp && \
    curl -L -O https://github.com/protocolbuffers/protobuf/releases/download/v${PROTOC_VERSION}/protoc-${PROTOC_VERSION}-linux-x86_64.zip && \
    unzip protoc-*.zip -d /usr/local && rm protoc-*.zip

WORKDIR /usr/src

RUN cargo install --path .

FROM base

RUN useradd -u 2000 orchestr8 -m -g 0

ENV HOME=/home/orchestr8

RUN chmod -R g+rwx ${HOME}

COPY --from=build /usr/local/cargo/bin/fms-orchestr8 /usr/local/bin/fms-orchestr8
COPY config /config

# Run as non-root user by default
USER orchestr8

CMD fms-orchestr8