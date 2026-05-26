# exgent

A Rust workspace porting the [pi](https://pi.dev) agent harness to a self-contained CLI + TUI.

## What it does

`exgent` is an interactive coding agent that talks to LLM providers (OpenAI, Anthropic,
Google, etc.) and executes tools (`read`, `write`, `edit`, `bash`, `grep`, `ls`, `find`)
inside a project directory. It runs as a ratatui-based TUI in a terminal, a plain
line-by-line interactive mode without a TTY, and as a one-shot `--print` / `--json`
mode for piping into other tools.

## Quick start

```sh
cargo build --release
./target/release/exgent              # interactive TUI (or plain mode without a TTY)
./target/release/exgent --print "summarize the README"
./target/release/exgent --json "summarize the README"
./target/release/exgent --help
```

On first run, use `/auth` inside the TUI to configure a provider (API key or OAuth),
then `/model` to pick one. Config and sessions live in `~/.exgent/` by default
(override with `--config <path>`).

## Layout

```
exgent (bin) → exgent-tui → exgent-core → exgent-ai
```

- [crates/exgent-ai](crates/exgent-ai/) — provider adapters (OpenAI completions/responses,
  Anthropic, Google, OpenAI Codex, fake), model catalog, model discovery.
- [crates/exgent-core](crates/exgent-core/) — runtime state, agent loop, tool registry,
  session JSONL, auth/OAuth, settings, localization.
- [crates/exgent-tui](crates/exgent-tui/) — ratatui interactive mode + plain fallback.
- [crates/exgent](crates/exgent/) — CLI parsing and process entry point.

See [CLAUDE.md](CLAUDE.md) for architectural notes.

## Development

```sh
cargo build                                  # debug
cargo test                                   # all tests
cargo test -p exgent-core <test-name>        # one test
cargo clippy --workspace --all-targets       # lint
cargo fmt --all                              # format
```

## License

MIT. See [LICENSE](LICENSE).
