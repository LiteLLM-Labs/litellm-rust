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

## Source of truth

- **Code conventions / architecture rules:** [`CODING_STANDARDS.md`](./CODING_STANDARDS.md) is authoritative — read it before any implementation change.
- **Layered request flow & module map:** [`docs/architecture.md`](./docs/architecture.md).

## Task completion

Before declaring a change done, run the same gate CI enforces (`.github/workflows/rust-checks.yml`):

```bash
cargo fmt --all --check
cargo check --all-targets --locked
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
python3 scripts/check_code_size.py   # ≤300 lines/file, ≤50 LOC/function
```

Commit messages follow conventional-commits prefixes (feat/fix/chore/docs).
