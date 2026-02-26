# Repository Guidelines

## Project Structure & Module Organization
- `src/main.rs`: application entrypoint and current core logic (Telegram bot, filesystem watcher, AI provider call, link execution).
- `Cargo.toml`: Rust package metadata and dependencies.
- `Cargo.lock`: locked dependency graph; keep this committed for reproducible builds.
- `config.yaml`: local runtime configuration (provider, prompt, watch tasks).
- `CONCEPT.md`: product behavior and workflow definition.
- `target/`: build artifacts (generated; do not edit).

As the project grows, split `main.rs` into modules such as `config.rs`, `watcher.rs`, `provider/`, and `linker.rs`.

## Build, Test, and Development Commands
- `cargo check`: fast compile validation without producing binaries.
- `cargo build`: build debug binary.
- `cargo run`: run locally using current env vars and `config.yaml`.
- `cargo fmt`: format code with Rustfmt.
- `cargo test`: run unit/integration tests.

Example:
```bash
KATAILINK_CHAT_ID=123456 TELOXIDE_TOKEN=... cargo run
```

## Coding Style & Naming Conventions
- Follow Rust defaults: 4-space indentation, `snake_case` for functions/variables, `CamelCase` for types, `SCREAMING_SNAKE_CASE` for constants.
- Always run `cargo fmt` before opening a PR.
- Prefer small, focused functions with explicit error context (`anyhow::Context`).
- Keep logs actionable (`info` for normal flow, `warn/error` for failures).

## Testing Guidelines
- Use Rust’s built-in test framework (`#[test]` and `#[tokio::test]`).
- Place unit tests near code (`mod tests`) and integration tests under `tests/` when introduced.
- Prioritize tests for:
  - filename/path generation
  - subtitle language tag mapping
  - AI output parsing and retry behavior

## Commit & Pull Request Guidelines
No commit history exists yet, so use a clear convention going forward:
- Commit format: `type(scope): summary` (e.g., `feat(watcher): dedupe duplicate notify events`).
- Keep commits atomic; avoid mixing refactor and behavior changes.
- PRs should include:
  - purpose and behavior changes
  - config/env changes (if any)
  - test/verification commands run (e.g., `cargo check`, `cargo test`)

## Security & Configuration Tips
- Never commit real secrets (bot token, private chat IDs).
- Use environment variables for runtime secrets: `TELOXIDE_TOKEN`, `KATAILINK_CHAT_ID`, optional `KATAILINK_CONFIG`.
- Validate watch/destination paths carefully before deploying to production media libraries.
