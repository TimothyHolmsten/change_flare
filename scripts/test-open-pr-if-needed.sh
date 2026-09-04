#!/usr/bin/env bash
# Offline tests for scripts/open-pr-if-needed.sh. No GitHub network calls.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCRIPT="$ROOT/scripts/open-pr-if-needed.sh"
PASS=0
FAIL=0

run_case() {
  local name="$1"
  local expect_exit="$2"
  local expect_stdout="$3"
  shift 3

  local tmp
  tmp="$(mktemp -d)"
  local mock="$tmp/mock-gh"
  local log="$tmp/calls.log"
  local out="$tmp/stdout"
  local err="$tmp/stderr"

  cat >"$mock" <<'MOCK'
#!/usr/bin/env bash
set -euo pipefail
echo "$*" >>"${MOCK_GH_LOG}"
args="$*"
case "$args" in
  "repo view "*" --json defaultBranchRef --jq .defaultBranchRef.name")
    printf '%s\n' "${MOCK_DEFAULT_BRANCH:-main}"
    ;;
  "pr list --repo "*" --head "*" --state open --json number --jq "*)
    printf '%s\n' "${MOCK_EXISTING_PR:-}"
    ;;
  "api repos/"*"/compare/"*)
    printf '%s\n' "${MOCK_AHEAD_BY:-1}"
    ;;
  "api repos/"*"/commits/"*" --jq .sha")
    printf '%s\n' "${MOCK_SHA:-abc123}"
    ;;
  "api repos/"*"/commits/"*" --jq .commit.message")
    printf '%s\n' "${MOCK_COMMIT_MSG:-feat: example change}"
    ;;
  "pr create "*)
    if [[ "${MOCK_CREATE_FAIL:-}" == "1" ]]; then
      echo "GraphQL: Resource not accessible by integration (createPullRequest)" >&2
      exit 1
    fi
    printf '%s\n' "${MOCK_PR_URL:-https://github.com/example/repo/pull/99}"
    ;;
  *)
    echo "unexpected gh invocation: $args" >&2
    exit 99
    ;;
esac
MOCK
  chmod +x "$mock"

  local exit_code=0
  env \
    GH_BIN="$mock" \
    MOCK_GH_LOG="$log" \
    GITHUB_REPOSITORY="TimothyHolmsten/change_flare" \
    GITHUB_REF_NAME="cursor/example-c390" \
    GITHUB_SHA="deadbeef" \
    "$@" \
    bash "$SCRIPT" >"$out" 2>"$err" || exit_code=$?

  if [[ "$exit_code" -ne "$expect_exit" ]]; then
    echo "FAIL $name (exit $exit_code, expected $expect_exit)"
    echo "stdout: $(cat "$out")"
    echo "stderr: $(cat "$err")"
    FAIL=$((FAIL + 1))
    rm -rf "$tmp"
    return
  fi

  if [[ -n "$expect_stdout" ]] && ! grep -F -- "$expect_stdout" "$out" >/dev/null; then
    echo "FAIL $name (stdout missing: $expect_stdout)"
    echo "stdout: $(cat "$out")"
    echo "stderr: $(cat "$err")"
    FAIL=$((FAIL + 1))
    rm -rf "$tmp"
    return
  fi

  echo "PASS $name"
  PASS=$((PASS + 1))
  rm -rf "$tmp"
}

run_case "skips default branch" 0 "Skipping: main is the default branch" \
  GITHUB_REF_NAME=main

run_case "skips existing open PR" 0 "Skipping: open pull request #7 already exists" \
  MOCK_EXISTING_PR=7

run_case "skips when not ahead of base" 0 "Skipping: cursor/example-c390 has no commits ahead of main" \
  MOCK_AHEAD_BY=0

run_case "creates PR when ahead and none exists" 0 "Opened https://github.com/example/repo/pull/99" \
  MOCK_AHEAD_BY=1 MOCK_EXISTING_PR=

run_case "dry run does not create" 0 "DRY_RUN: would create PR" \
  DRY_RUN=1 MOCK_AHEAD_BY=1

# Missing env should fail
tmp="$(mktemp -d)"
if GITHUB_REPOSITORY= GITHUB_REF_NAME= bash "$SCRIPT" >"$tmp/out" 2>"$tmp/err"; then
  echo "FAIL missing env should exit non-zero"
  FAIL=$((FAIL + 1))
else
  if grep -q "GITHUB_REPOSITORY and GITHUB_REF_NAME must be set" "$tmp/err"; then
    echo "PASS missing env"
    PASS=$((PASS + 1))
  else
    echo "FAIL missing env message"
    cat "$tmp/err"
    FAIL=$((FAIL + 1))
  fi
fi
rm -rf "$tmp"

echo
echo "$PASS passed, $FAIL failed"
if [[ "$FAIL" -ne 0 ]]; then
  exit 1
fi
