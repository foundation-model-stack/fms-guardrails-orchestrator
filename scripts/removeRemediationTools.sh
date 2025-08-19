#!/usr/bin/env bash

echo "removing remediation tools"
microdnf remove -y gnutls-utils shadow-utils
echo "remediation tool removal complete"