#/usr/bin/env sh

rustc --version
echo "Commit hash: $(git log -1 --pretty=format:'%h')"