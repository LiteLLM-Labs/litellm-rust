# AGENTS.md

Before making implementation changes, read and follow the repo-wide
[`CODING_STANDARDS.md`](./CODING_STANDARDS.md).

## First-time setup

Run once after cloning to activate the committed git hooks:

```bash
git config core.hooksPath .githooks
```

The pre-commit hook keeps `model_prices_backup.json` in sync with the
upstream litellm JSON on every commit. It warns and skips silently if
the network is unavailable — it never blocks a commit.
