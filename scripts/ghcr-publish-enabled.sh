#!/usr/bin/env bash
# Print true when this GitHub Actions event should push the image to GHCR.
# Reads GITHUB_EVENT_NAME and GITHUB_REF. Always exits 0.
set -euo pipefail

event="${GITHUB_EVENT_NAME:-}"
ref="${GITHUB_REF:-}"
publish=false

if [ "${event}" = "push" ] || [ "${event}" = "workflow_dispatch" ]; then
  if [ "${ref}" = "refs/heads/main" ] || [[ "${ref}" == refs/tags/v*.*.* ]]; then
    publish=true
  fi
fi

printf '%s\n' "${publish}"
