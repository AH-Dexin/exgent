# Changelog

All notable changes to this workspace are documented here. Each crate also has
its own changelog under [crates/*/CHANGELOG.md](crates/) that captures changes
specific to that package. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Added
- `CancelToken` threaded through `AgentSession::run_prompt_events_cancellable`,
  the agent loop, and the `bash` tool (Unix uses `setsid` + process-group kill).
- Three new builtin tools — `ls`, `grep`, `find` — sharing a common walker that
  skips VCS and build directories.
- `AgentHooks` / `SharedAgentHooks` for embedders to short-circuit or rewrite
  tool calls and transform outgoing messages.
- `KeyBindings` in `settings.json` with `KeyAction` enum for configurable
  default key strokes.
- `RuntimeOptions::agent` (`AgentLoopConfig`) makes `max_tool_rounds`
  configurable and replaces the previous `MAX_TOOL_ROUNDS = 8` constant.
- `RuntimeOptions::enable_dev_providers` enables the `fake` provider for
  integration tests.
- `Storage` trait with `FsStorage` and `InMemoryStorage` implementations.
- Structured error enums via `thiserror`: `PersistenceError`,
  `ModelServiceError`, `AgentSessionError`.
- `sdk` module re-exports the curated embedding surface.
- GitHub Actions CI: `cargo fmt --check`, `cargo clippy -D warnings`,
  `cargo test` on Linux/macOS/Windows.
- Integration test harness under `crates/exgent-core/tests/`.

### Changed
- Compaction summary now lists the files that were read and modified within
  the compacted window.
- `ToolRegistry` and the `Tool` trait are now public so third-party crates can
  register custom tools.
- `pub use runtime::*` replaced with an explicit re-export list to lock down
  the public API surface.
