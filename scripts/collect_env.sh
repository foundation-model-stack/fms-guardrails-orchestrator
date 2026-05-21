#/usr/bin/env sh

rustc --version

ORCHESTRATOR_VERSION=$(git tag --points-at HEAD)
if [ -z "$ORCHESTRATOR_VERSION" ]; then
    ORCHESTRATOR_VERSION="No tag associated"
fi
echo "Orchestrator version: $ORCHESTRATOR_VERSION ($(git log -1 --pretty=format:'%h'))"