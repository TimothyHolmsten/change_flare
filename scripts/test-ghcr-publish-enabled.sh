#!/usr/bin/env bash
# Offline tests for scripts/ghcr-publish-enabled.sh.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCRIPT="$ROOT/scripts/ghcr-publish-enabled.sh"
PASS=0
FAIL=0

run_case() {
  local name="$1"
  local event="$2"
  local ref="$3"
  local expect="$4"
  local actual
  actual="$(GITHUB_EVENT_NAME="${event}" GITHUB_REF="${ref}" bash "${SCRIPT}")"
  if [ "${actual}" = "${expect}" ]; then
    echo "PASS ${name}"
    PASS=$((PASS + 1))
  else
    echo "FAIL ${name} (got ${actual}, expected ${expect})"
    FAIL=$((FAIL + 1))
  fi
}

run_case "push main" push refs/heads/main true
run_case "workflow_dispatch main" workflow_dispatch refs/heads/main true
run_case "version tag" push refs/tags/v0.2.2 true
run_case "prerelease tag" push refs/tags/v0.2.2-rc.1 true
run_case "schedule on main" schedule refs/heads/main false
run_case "pull request" pull_request refs/pull/12/merge false
run_case "push feature branch" push refs/heads/cursor/example-d1ea false
run_case "dispatch feature branch" workflow_dispatch refs/heads/cursor/example-d1ea false
run_case "non-semver tag" push refs/tags/latest false
run_case "short tag" push refs/tags/v1.2 false
run_case "missing env" "" "" false

echo "${PASS} passed, ${FAIL} failed"
[ "${FAIL}" -eq 0 ]
