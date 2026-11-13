# ADR 0002: Rust core with a GPUI UI; first adapters are Claude Code, Codex, Antigravity

Status: accepted

## Decision

- **Language:** Rust. The core and the UI run in one process. We chose speed
  over borrowing t3code's TypeScript code. We still use t3code as a reference
  design.
- **UI:** [GPUI Kit](https://gpui-kit.com) (`gpui-kit = "0.9.1"`). It
  bundles `gpui-pre`, a November 2026 snapshot of Zed's GPUI, together with
  `gpui-component` (75+ components), `gpui-base`, and assets. We don't use
  the official `gpui` 0.2.2 crate: it dates from October 2025 and breaks with
  the current `libc`. Trade-off: GPUI Kit is maintained by Longbridge, not
  Zed. Its `speech` feature captures the microphone on all platforms
  (`cpal`), but it only transcribes speech on macOS. On Linux we need our own
  speech-to-text (for example whisper.cpp).
- **First adapters:** Claude Code, Codex, and Antigravity. Each one is driven
  over stdio and launched with the shift MCP server injected.

| Harness     | Launch                                                                        | Protocol                                             | Instructions / limits                                                                                     | MCP injection                                        |
|-------------|-------------------------------------------------------------------------------|------------------------------------------------------|-----------------------------------------------------------------------------------------------------------|------------------------------------------------------|
| Claude Code | `claude -p --input-format stream-json --output-format stream-json --verbose`  | NDJSON over stdio                                    | `--append-system-prompt`, `--allowedTools`, `--disallowedTools`, `--permission-mode`, `--model`            | `--mcp-config <json> --strict-mcp-config`            |
| Codex       | `codex app-server`                                                            | JSON-RPC over stdio (`codex app-server generate-json-schema`) | `-c` config overrides: model, sandbox, approval policy, developer instructions                             | MCP server config in `thread/start`                  |
| Antigravity | `agy acp`                                                                     | ACP over stdio (`agent-client-protocol` crate)       | `session/new` params. shift answers `session/request_permission`, which is where limits get enforced       | `mcpServers` in `session/new`                        |

Antigravity goes through ACP, so building that adapter also gives us OpenCode
and other ACP agents almost for free.

## Consequences

- We build the shift MCP server ourselves in Rust. Candidate crate: `rmcp`.
- Codex has no published Rust protocol crate. We generate the message types
  from its JSON Schema output, or write them by hand.
- Each adapter gets an integration test that launches the real binary,
  gated behind an environment variable because these runs use real accounts.
