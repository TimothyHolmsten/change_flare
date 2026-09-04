#!/usr/bin/env bash
# Open a GitHub pull request for the current branch when one does not already
# exist. Used by .github/workflows/open-cursor-pr.yml.
#
# Cloud Agent GitHub App tokens often cannot call createPullRequest (403
# "Resource not accessible by integration"). GitHub Actions GITHUB_TOKEN can.
set -euo pipefail

gh_bin="${GH_BIN:-gh}"

gh() {
  "$gh_bin" "$@"
}

repo="${GITHUB_REPOSITORY:-}"
branch="${GITHUB_REF_NAME:-}"
sha="${GITHUB_SHA:-}"

if [[ -z "$repo" || -z "$branch" ]]; then
  echo "GITHUB_REPOSITORY and GITHUB_REF_NAME must be set" >&2
  exit 1
fi

owner="${repo%%/*}"

default_branch="$(
  gh repo view "$repo" --json defaultBranchRef --jq .defaultBranchRef.name
)"

if [[ -z "$default_branch" ]]; then
  echo "Could not determine default branch for $repo" >&2
  exit 1
fi

if [[ "$branch" == "$default_branch" ]]; then
  echo "Skipping: $branch is the default branch"
  exit 0
fi

existing="$(
  gh pr list \
    --repo "$repo" \
    --head "${owner}:${branch}" \
    --state open \
    --json number \
    --jq '.[0].number // empty'
)"

if [[ -n "$existing" ]]; then
  echo "Skipping: open pull request #$existing already exists for $branch"
  exit 0
fi

ahead_by="$(
  gh api "repos/${repo}/compare/${default_branch}...${branch}" --jq .ahead_by
)"

if [[ "${ahead_by:-0}" -eq 0 ]]; then
  echo "Skipping: $branch has no commits ahead of $default_branch"
  exit 0
fi

if [[ -z "$sha" ]]; then
  sha="$(gh api "repos/${repo}/commits/${branch}" --jq .sha)"
fi

title="$(
  gh api "repos/${repo}/commits/${sha}" --jq '.commit.message' | head -n 1
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

url="$(
  gh pr create \
    --repo "$repo" \
    --base "$default_branch" \
    --head "$branch" \
    --title "$title" \
    --body "$body"
)"

echo "Opened $url"
