# AGENTS.md

Guidance for coding agents working in this repository.

## GitHub pull requests

Cursor Cloud Agent GitHub App tokens **cannot create pull requests**. `gh pr create` fails with `403 Resource not accessible by integration`, and the UI shows "GitHub rejected the pull request. Check your branches and try again." Git push still works. This is a token-scope issue, not a missing or mismatched branch.

Do this instead:

1. Commit and push the `cursor/...` branch. Do not retry `gh pr create` after a 403.
2. Open or update the PR with Cursor's **ManagePullRequest** tool.
3. If that tool is unavailable, `.github/workflows/open-cursor-pr.yml` opens a PR on push to `cursor/**` using Actions `GITHUB_TOKEN`.

Optional (recommended so auto-opened PRs also run CI): add a repository secret `PR_CREATE_TOKEN` — a fine-grained PAT with **Pull requests: write** and **Contents: read** on this repo. PRs created with the default `GITHUB_TOKEN` do not trigger further Actions workflows.

Optional (so `gh` inside a Cloud Agent can create PRs): add the same PAT as a Cloud Agent environment secret named `GH_TOKEN`.
