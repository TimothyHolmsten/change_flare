#!/usr/bin/env bash
# Open a GitHub pull request for the current branch when one does not already
# exist. Used by .github/workflows/open-cursor-pr.yml.
#
# Cloud Agent GitHub App tokens often cannot call createPullRequest (403
# "Resource not accessible by integration"). GitHub Actions GITHUB_TOKEN can.
set -euo pipefail

# Never name a function `gh` that execs `$GH_BIN` defaulting to `gh` — that
# recurses until bash SIGSEGV (exit 139) when GH_BIN is unset in Actions.
run_gh() {
  if [[ -n "${GH_BIN:-}" ]]; then
    "$GH_BIN" "$@"
  else
    command gh "$@"
  fi
}

repo="${GITHUB_REPOSITORY:-}"
branch="${GITHUB_REF_NAME:-}"
sha="${GITHUB_SHA:-}"

if [[ -z "$repo" || -z "$branch" ]]; then
  echo "GITHUB_REPOSITORY and GITHUB_REF_NAME must be set" >&2
  exit 1
fi

default_branch="$(
  run_gh repo view "$repo" --json defaultBranchRef --jq .defaultBranchRef.name
)"

if [[ -z "$default_branch" ]]; then
  echo "Could not determine default branch for $repo" >&2
  exit 1
fi

if [[ "$branch" == "$default_branch" ]]; then
  echo "Skipping: $branch is the default branch"
  exit 0
fi

# Use --head BRANCH, not OWNER:BRANCH. The latter returns no rows for this repo.
existing="$(
  run_gh pr list \
    --repo "$repo" \
    --head "$branch" \
    --state open \
    --json number \
    --jq '.[0].number // empty'
)"

if [[ -n "$existing" ]]; then
  echo "Skipping: open pull request #$existing already exists for $branch"
  exit 0
fi

ahead_by="$(
  run_gh api "repos/${repo}/compare/${default_branch}...${branch}" --jq .ahead_by
)"

if [[ "${ahead_by:-0}" -eq 0 ]]; then
  echo "Skipping: $branch has no commits ahead of $default_branch"
  exit 0
fi

if [[ -z "$sha" ]]; then
  sha="$(run_gh api "repos/${repo}/commits/${branch}" --jq .sha)"
fi

title="$(
  run_gh api "repos/${repo}/commits/${sha}" --jq '.commit.message' | head -n 1
)"
title="${title:-${branch}}"

body="$(
  cat <<EOF
Automated pull request opened on push of \`${branch}\`.

Cursor Cloud Agent tokens cannot create GitHub pull requests (\`403 Resource not accessible by integration\`). This workflow uses the Actions \`GITHUB_TOKEN\` so a PR still appears after the branch is pushed.

Head: \`${branch}\` (${sha})
Base: \`${default_branch}\`
EOF
)"

if [[ "${DRY_RUN:-}" == "1" ]]; then
  echo "DRY_RUN: would create PR"
  echo "title: $title"
  echo "base: $default_branch"
  echo "head: $branch"
  exit 0
fi

set +e
url="$(
  run_gh pr create \
    --repo "$repo" \
    --base "$default_branch" \
    --head "$branch" \
    --title "$title" \
    --body "$body" 2>&1
)"
create_status=$?
set -e

if [[ "$create_status" -ne 0 ]]; then
  if [[ "$url" == *"already exists"* ]]; then
    echo "Skipping: $url"
    exit 0
  fi
  echo "$url" >&2
  exit "$create_status"
fi

echo "Opened $url"
