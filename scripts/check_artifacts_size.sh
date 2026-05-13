#!/usr/bin/env bash

set -e
set -o pipefail

# Juno wasmd default MaxWasmCodeSize is ~3 MB (3 * 1024 * 1024 bytes).
# Upstream Astroport pinned this to 800 KB to match Terra's and Injective's
# tighter wasmd limits; that ceiling doesn't apply on Juno. We keep some
# headroom margin under Juno's real limit so artifacts don't surprise the
# governance upload path.
maximum_size=3072

for artifact in artifacts/*.wasm; do
  artifactsize=$(du -k "$artifact" | cut -f 1)
  if [ "$artifactsize" -gt $maximum_size ]; then
    echo "Artifact file size exceeded: $artifact"
    echo "Artifact size: $artifactsize"
    echo "Max size: $maximum_size"
    exit 1
  fi
done
