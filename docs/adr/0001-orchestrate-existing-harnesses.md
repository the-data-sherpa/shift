# ADR 0001: Orchestrate existing harnesses instead of calling models directly

Status: proposed

## Context

We had two options:

1. **Own the agent loop.** shift calls model APIs directly and implements
   tools, sandboxing, and context management itself.
2. **Orchestrate harnesses.** shift launches instances of existing harnesses
   (Codex, Claude Code, Antigravity, Pi, Oh My Pi, OpenCode). Each instance is
   configured from a shift agent definition, and shift hooks into it.

## Decision

Option 2. shift is an orchestrator. It doesn't run the agent loop itself.

Reasons:

- **Subscriptions.** The harnesses run on the plans we already pay for. Calling
  the APIs directly means paying per token on top of those plans.
- **Tooling.** Each harness already has file editing, shell access,
  sandboxing, approvals, compaction, and its own native subagents. Rebuilding
  all of that is a project in itself.
- **Model strengths.** Different harnesses are good at different jobs. With
  cross-harness delegation, each role can run wherever it works best.
- **Proven pattern.** t3code ships this design (see below).

## Reference: how t3code does it

- The app runs a local **MCP server** (`/mcp` on 127.0.0.1) that exposes
  orchestration tools: `orchestrator_capabilities`, `delegate_task`,
  `task_status`, `task_cancel`, and thread send/read/wait.
- Before each provider session starts, the app issues a **bearer token
  scoped to that session**. The token is revoked when the session ends.
- The MCP server is **injected into each harness** at launch, using that
  harness's own mechanism:
  - Codex: the `codex app-server` `thread/start` config.
  - Claude: `mcpServers` in the Agent SDK options, plus
    `allowedTools: ["mcp__t3-code__*"]`.
  - ACP agents (Antigravity, Grok, OpenCode): `mcpServers` in
    `session/new`.
  - Pi: `pi --mode rpc --extension <bridge>`, with the URL and token set as
    environment variables.
- `delegate_task` creates an **app-owned child thread** on any provider and
  model. It runs in `async` or `wait` mode. The child receives **only the task
  prompt and a role**, not the parent's history.
- **Privileges only narrow.** A child's runtime mode and interaction mode can
  equal the parent's or be stricter, never broader.
- `clientRequestId` makes retries idempotent.
- When a child asks a question, the question is forwarded to the parent
  instead of sitting unanswered in the child thread.
- Each adapter reports **capabilities**: forking, steering, native subagents,
  and so on. Orchestration policy branches on those capabilities, not on
  provider names.

Source: `pingdotgg/t3code`, `docs/orchestration-v2/orchestrator-mcp-server.md`.

## Consequences for shift

- **The shift MCP server is the bus ingress.** Agents publish findings, hand
  off work, and delegate by calling shift tools. Every message goes through
  shift, so the UI and the meta thread see all of it.
- **Each harness gets an adapter.** An adapter has two jobs:
  1. **Compile** a shift agent definition into the harness's launch config:
     system prompt, model, tool allow/deny lists, sandbox mode, MCP servers,
     and skills.
  2. **Drive** the session: start, send, stream events, interrupt, resume.
- **Capability boundaries depend on what each harness can enforce.** Each
  harness has different controls:
  - Claude: `allowedTools` and `disallowedTools`.
  - Codex: sandbox mode and approval policy.
  - ACP: the client answers `session/request_permission`, which gives shift a
    clean place to enforce limits.
  - Pi: extensions.

  Some roles may not be enforceable on some harnesses. In that case the
  adapter has to **refuse to launch that role** there rather than weaken the
  boundary silently.
- **Language stays open.** Every integration point works over stdio JSON-RPC
  or the CLI: Codex app-server, `claude -p --input-format stream-json
  --output-format stream-json`, ACP, and Pi RPC mode. Nothing requires
  TypeScript, so a Rust core with a GPUI front end is still possible. Only the
  Claude *Agent SDK* is limited to TypeScript and Python. The Claude CLI's
  stream-json mode avoids that dependency.
- **The meta thread can be a harness instance too.** It would be a narrow
  "router" agent whose only tools are the shift orchestration tools.
