# ADR 0003: Harness adapters

Status: proposed

## Context

An **adapter** is shift's integration with one backend harness (see
`CONTEXT.md`). It has three jobs:

1. **Launch:** turn a shift agent definition into that harness's launch
   command and session config.
2. **Talk:** send messages in, and translate the harness's output into
   shift's common `Event` stream (text, tool call, tool result, permission
   request, turn done, error).
3. **Enforce:** apply the agent's capability boundary using the controls the
   harness offers.

The UI, the meta thread, flows, and the bus only see the shared
`Session`/`Event` interface. Nothing outside an adapter contains
harness-specific code.

## Decision

### 1. Each adapter speaks its harness's native protocol in Rust, over stdio

| Harness     | Launch           | Protocol                                                                 | Rust approach                                                        |
|-------------|------------------|--------------------------------------------------------------------------|----------------------------------------------------------------------|
| Claude Code | `claude -p --input-format stream-json --output-format stream-json --verbose` | NDJSON over stdio                                                        | Hand-written types for the messages we use. No Node dependency.       |
| Codex       | `codex app-server` | JSON-RPC over stdio (`thread/start`, `turn/start`, `item/*` notifications) | Hand-written types for the subset we use, checked against `codex app-server generate-json-schema`. |
| Antigravity | `agy acp`        | ACP (`initialize`, `session/new`, `session/prompt`, `session/update`)     | `agent-client-protocol` crate.                                         |

We are **not** using ACP wrappers for Claude and Codex
(`@agentclientprotocol/claude-agent-acp`, `@agentclientprotocol/codex-acp`)
as the main path:

- Both are TypeScript, so the Rust app would need Node.
- Each adds a layer that tends to lag behind the harness it wraps.
- They hide native features we want, such as Codex thread fork and Claude
  session resume.

They remain a fallback. The ACP adapter we build for Antigravity also covers
OpenCode and other ACP agents later.

### 2. Enforce limits twice: at launch and at runtime

| Harness     | Launch-time limit                                               | Runtime approval by shift                                                              |
|-------------|-----------------------------------------------------------------|----------------------------------------------------------------------------------------|
| Claude Code | `--tools <list>` removes every built-in tool not listed           | `--permission-prompts host`: shift (the host) answers each prompt over stdio            |
| Codex       | `sandbox` in `thread/start` (`read-only` / `workspace-write`)    | `approvalPolicy`: shift answers `item/commandExecution/requestApproval` and `item/fileChange/requestApproval` |
| Antigravity | System prompt via `_meta.antigravity.systemPrompt.append`        | ACP `session/request_permission`: shift is the client and answers each one              |

Every adapter sends its harness's approval requests to one shared **policy
gate** in `shift-core`. The gate checks each request against the agent
definition's `tools` and `forbidden` lists. The capability rules live in one
place, and they hold even when a launch-time setting is imperfect.

If a harness cannot express a role's boundary, its adapter **refuses to
launch** that role there instead of quietly weakening the boundary.

### 3. The MCP server is injected through a stdio bridge

shift ships one subcommand: `shift mcp-bridge --session <token>`. Every
harness launches it as a stdio MCP server, and it forwards calls to the shift
core. The token is scoped to that one session and revoked when the session
ends.

- All three harnesses can launch stdio MCP servers. ACP requires stdio
  support; HTTP support is optional.
- Antigravity's `initialize` response (1.5.8) did not advertise
  `mcpCapabilities.http`, so stdio is the safe common choice.
- Injection points:
  - Claude: `--mcp-config <json> --strict-mcp-config`
  - Codex: MCP server config in `thread/start`
  - Antigravity: `mcpServers` in `session/new`

### 4. Build order

1. **Claude Code.** It has the simplest protocol: one NDJSON stream with a
   clear `result` message that ends each turn.
2. **Codex.** This completes the first cross-harness delegation demo
   (Claude ↔ Codex).
3. **Antigravity.** The ACP crate does most of the work, and this adapter
   also opens up OpenCode and other ACP agents.

### 5. Guard against harness changes

These protocols change often. Codex's app-server is still marked
experimental. At the time of writing we have `codex-cli` 0.167.0,
Claude Code 2.1.319, and `agy` 1.5.8.

- **Version check:** each adapter declares the harness versions it was tested
  against and checks the installed version at launch. An untested version
  gets a warning, not a silent failure.
- **Recorded-session tests:** real sessions are captured as NDJSON or
  JSON-RPC transcripts and replayed in unit tests, so tests are fast and need
  no account.
- **Live smoke tests:** these launch the real binaries and are gated behind an
  environment variable, because they use real accounts.

## Still to verify

Check each item at the start of the matching adapter's work:

- **Claude:** the exact message shapes for permission prompts with
  `--permission-prompts host` in stream-json mode. The flag exists; we have
  not checked the message format.
- **Antigravity:** whether it accepts HTTP MCP servers. We inferred "no" from
  the missing capability. The stdio bridge works either way.
- **Antigravity:** whether it accepts a per-session model and a per-session
  tool restriction through ACP or `_meta`.
- **Codex:** which `approvalPolicy` value sends every write and command to
  the client, and how the MCP server is configured in `thread/start`.

## Consequences

- `shift-core` gets the shared `Adapter`, `Session`, and `Event` types and the
  policy gate.
- Each adapter lives in its own crate:
  - `shift-adapter-claude`
  - `shift-adapter-codex`
  - `shift-adapter-acp` (Antigravity first)
- Agent definitions use shift tool names such as `fs.read` and `fs.write`.
  Each adapter maps them to its harness's own tool names and permission
  categories.
