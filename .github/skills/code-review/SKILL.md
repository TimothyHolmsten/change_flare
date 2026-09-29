---
name: code-review
description: Review change_flare pull requests for correctness, reliability, security, and compatibility with the repository's DDNS and Cloudflare API invariants.
---

# Code review

Use this skill when reviewing changes to `change_flare`.

## Review process

1. Read `AGENTS.md`, the pull request description, and the complete diff.
2. Trace changed behavior through its callers and tests instead of reviewing lines in isolation.
3. Check error paths, shutdown behavior, retries, configuration validation, and observable logging.
4. Check for credential disclosure, unsafe DNS updates, accidental changes to unrelated record types, and network or input handling vulnerabilities.
5. Verify tests cover changed behavior and follow the repository's `mockito` and production-URL conventions.
6. Report only actionable findings introduced by the change, ordered by severity. Include the file, line, impact, and a concise remediation.

## Repository invariants

- This is an origin DDNS agent, not a Cloudflare Worker.
- Keep `ureq` 3 as the sole HTTP stack and reuse a long-lived `Agent`.
- Update DNS records with `PATCH` and `{ "content": "<ip>" }`; preserve all other record properties.
- List only enabled `A`/`AAAA` families, paginate results, and use exact record-name filtering when configured.
- Never rewrite CNAME, MX, TXT, or other record types.
- Prefer `CLOUDFLARE_RECORD_NAMES`; warn before updating every address record in a zone.
- Use bearer API tokens and never expose credentials in logs, tests, or documentation.
- Retry only Cloudflare `429`, `502`, `503`, and `504` responses. Sleep before every retry, honor `Retry-After` up to 30 seconds, and preserve interruptible shutdown behavior.
- Enforce the 60-second minimum poll interval and avoid busy loops after STUN or API failures.
- Reject non-globally-routable STUN results before they reach DNS.
- Preserve Rust edition 2024 and MSRV 1.88; library code must not introduce `unwrap`, `expect`, or `unreachable`.

## Review output

Do not make edits while reviewing unless explicitly asked to implement the findings. Do not report style preferences or pre-existing issues. If no actionable issue is found, state that clearly and mention any meaningful testing limitations.
