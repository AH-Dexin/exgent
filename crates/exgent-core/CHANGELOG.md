# exgent-core changelog

## [Unreleased]

### Added
- `CancelToken` exposed through `AppRuntimeHost::run_prompt_events_cancellable`.
  Agent loop and tools (`bash` in particular) check the token at safe
  boundaries.
- `AgentHooks` trait + `SharedAgentHooks` alias for embedders that need to
  intercept tool calls or transform messages before they go to the provider.
- `KeyBindings` and `KeyAction` types persisted in `settings.json`.
- `AgentLoopConfig` (in `RuntimeOptions::agent`) makes `max_tool_rounds`
  configurable.
- `RuntimeOptions::enable_dev_providers` opts into the `fake` adapter.
- `Storage` trait + `FsStorage` / `InMemoryStorage` impls.
- Public `Tool`, `ToolOutput`, `ToolRegistry`.
- `sdk` module bundles the minimal embedder surface.
- Built-in tools: `ls`, `grep`, `find` (all execute in parallel mode).
- Compaction summary tracks files read/modified.
- Structured error enums (`PersistenceError`, `ModelServiceError`,
  `AgentSessionError`).
- Integration tests under `tests/agent_session.rs`.

### Changed
- `pub use runtime::*` replaced with explicit re-exports.
