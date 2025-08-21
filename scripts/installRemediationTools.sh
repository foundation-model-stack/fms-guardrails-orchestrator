#!/usr/bin/env bash

echo "installing remediation tools"
microdnf install -y crypto-policies crypto-policies-scripts \
        gnutls-utils subscription-manager \
        passwd shadow-utils util-linux pam \
        findutils xz
echo "remediation tool installation complete"
microdnf clean all
echo "complete clean all"