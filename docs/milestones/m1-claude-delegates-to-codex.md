# M1: Claude delegates validation to Codex

Status: in progress

## Goal

In the shift app, you give Claude Code a task. Claude does the work, then
delegates validation to Codex. Codex checks the work on its own and returns a
verdict. Claude acts on that verdict. You watch both threads live.

This is the foundation for everything after it. It tests the parts of shift
that are new: conversation-driven work and agent-initiated delegation across
harnesses. It also tests whether narrow agents are worth it: a validator on a
different harness and model, with none of the coder's context.

## Done when

You use it for a **small real task in one of your repos**, not a demo, and:

- Claude (`coder`) makes the change.
- Claude calls `delegate_task`. shift starts Codex (`validator`) in the same
  working directory, with the task brief and a description of the change but
  none of Claude's history.
- Codex runs the real checks and returns a structured verdict.
- Claude either fixes the reported problems and re-validates, or reports the
  result to you.
- The app shows both threads while they run.
- **One honest note:** did the validator catch something, or confirm
  something, that Claude alone would have missed? Write that in this file
  when M1 closes.

## Already done (foundation)

- Vision and vocabulary: `README.md`, `CONTEXT.md`.
- ADR 0001: orchestrate existing harnesses; don't call model APIs directly.
- ADR 0002: Rust core and a GPUI Kit UI. First harnesses: Claude Code, Codex,
  then Antigravity.
- ADR 0003: native protocol adapters, a shared policy gate, and MCP injection.
- Cargo workspace:
  - `shift-core` loads agent definitions (tests pass).
  - `shift-app` is a GPUI Kit window listing the agents.
- Example agent and flow definitions in `agents/` and `flows/`.
- Review of the earlier crewroom project: what to reuse and what to avoid.

## To build, in order

1. **Core types** in `shift-core`: `Adapter`, `Session`, `Event`, and a
   `DelegateTask` request/result.
2. **Claude Code adapter.** Launch from an agent definition. Send messages
   over stream-json. Translate output into `Event`s. Limit tools with
   `--tools`. Use the recorded-session approach (ADR 0003) for tests.
3. **Codex adapter.** Use `codex app-server`: `thread/start` with the
   validator's instructions, sandbox, and approval policy, then `turn/start`,
   with `item/*` notifications translated into `Event`s.
4. **`delegate_task` for Claude.** One tool. Claude calls it, and shift
   starts the validator on Codex and returns its verdict.
5. **Conversation UI.** A text input that sends to the coder, plus live views
   of the coder thread and the delegated validator thread.
6. **Real-task run.** Meet the "Done when" criteria above and record the
   honest note.

## Decisions to settle early

- **How Claude reaches `delegate_task`.** Crewroom answered tool calls through
  Claude's stream-json control channel (`control_request`/`mcp_message`, an
  in-process "SDK" MCP server). That needs no bridge process. It may be
  simpler than the `shift mcp-bridge` from ADR 0003 for M1. Check this and
  update ADR 0003 to match.
- **Blocking or async.** Does `delegate_task` block until Codex finishes?
  First find Claude Code's MCP tool-call timeout. A real validation can take
  minutes. If the timeout is too short, return a task ID immediately and add
  `task_status`.
- **How the validator runs tests.** Codex's `read-only` sandbox may block
  `cargo test` or `npm test`, which write build output. Options:
  - `workspace-write`, with source edits denied through approval handling
  - a scratch copy of the checkout

  Decide before step 3 is done.
- **Verdict shape.** A starting point: `{ verdict: pass|fail, findings[],
  commands_run[] }`. Keep it structured so Claude doesn't have to interpret
  free text.

## Out of scope for M1

Each of these is deferred, not rejected:

- voice input
- meta-thread intent routing (the coder → validator flow is hard-coded)
- Antigravity, Pi, Oh My Pi, and OpenCode
- the flow file format and a general message bus
- the full policy gate

## Reuse from crewroom

Copy and adapt; don't take in its workflow machinery.

- `crates/crewroom-runtime/src/runner/claude.rs`, `codex.rs`, `jsonl.rs`
  (about 1,800 lines together).
- `docs/research/claude-mh3-protocol-2026-10-13.md`: the stream-json control
  protocol, SDK MCP tool server, and pre-tool hook.
- `docs/research/codex-mh2-protocol-readiness-2026-10-12.md`: `thread/start`
  parameters and sandbox/approval behavior. It warns that a read-only
  sandbox is **not** a complete tool-deny policy.
- Known pitfall: commands started inside a Claude Code session inherit
  `RLIMIT_RTTIME=0`, and the kernel SIGKILLs them under CPU load (crewroom
  FOR-68). Remember this if tests die with no visible cause.

## Working rule

Keep this milestone lean. Don't add a new doc, setting, or abstraction unless
the M1 loop needs it.
