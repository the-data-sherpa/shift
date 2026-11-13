# M1: Claude delegates review to Codex

Status: in progress

## Goal

In the shift app, you give Claude Code a task. Claude makes the smallest
complete change, proves it, then delegates a targeted review to Codex running
Sol. The reviewer sees your request, the coder's intent, and the diff, but
none of Claude's history. Claude answers each finding (fix, disagree with
evidence, or decline) and re-delegates until the review approves or the
review is Stuck. You watch both sessions live.

This follows the `sol-loop` review contract, with shift as the orchestrator
instead of a skill. It tests the parts of shift that are new:
conversation-driven work and agent-initiated delegation across harnesses. It
also tests whether narrow agents are worth it: a reviewer on a different
harness and model, with none of the coder's context.

## Done when

You use it for a **small real task from shift's own backlog**, not a demo
(fallback: `~/Projects/brain`), and:

- Claude (`coder`) makes the change and runs focused checks.
- Claude calls `delegate_task`. shift snapshots the working tree into a
  temporary worktree and starts Codex (`reviewer`) there with the brief.
- Codex returns a schema-checked review.
- Claude answers each finding and re-delegates, or reports the final review
  (approve or Stuck) to you.
- The app shows both sessions while they run.
- **One honest note:** did the reviewer catch something that the coder's own
  checks had not already shown? Write that in this file when M1 closes.

## Already done (foundation)

- Vision and vocabulary: `README.md`, `CONTEXT.md`.
- ADR 0001: orchestrate existing harnesses; don't call model APIs directly.
- ADR 0002: Rust core and a GPUI Kit UI. First harnesses: Claude Code, Codex,
  then Antigravity.
- ADR 0003: native protocol adapters, a shared policy gate, and MCP injection.
- Cargo workspace:
  - `shift-core` loads agent definitions (tests pass).
  - `shift-app` is a GPUI Kit window listing the agents.
- Agent definitions `agents/coder.md` and `agents/reviewer.md`; example flows
  in `flows/`.
- Review of the earlier crewroom project: what to reuse and what to avoid.

## To build, in order

1. **Core types** in `shift-core`: `Adapter`, `Session`, `Event`, and the
   `delegate_task` request and review result.
2. **Claude Code adapter.** Launch from an agent definition. Send messages
   over stream-json. Translate output into `Event`s. Limit tools with
   `--tools`. Answer host permission prompts with the minimal policy gate.
   Serve `delegate_task` over the control channel. Early spike: a dummy tool
   that blocks for 10 minutes, to confirm blocking calls survive. Use
   recorded sessions (ADR 0003) for tests.
3. **Codex adapter.** Use `codex app-server`: `thread/start` with the
   reviewer's instructions, `workspace-write` sandbox in the snapshot
   worktree, and approval policy; `turn/start` with the review output
   schema; `item/*` notifications translated into `Event`s. Later rounds are
   new turns on the same thread.
4. **`delegate_task`.** Snapshot, brief, run the reviewer, validate the
   review against the schema, track rounds and the ledger, return the review.
5. **Conversation UI.** A text input that sends to the coder, a live coder
   pane, and a read-only reviewer pane with each finding's history.
6. **Real-task run.** Meet the "Done when" criteria above and record the
   honest note.

## Decisions settled

### Roles and contract

- **Reviewer, not validator.** The coder owns proof: it runs the checks and
  reports the results. The reviewer reads the diff and surrounding code, and
  may run a focused check only to confirm a finding.
- **Review contract** follows `sol-loop`: severities `blocker | should | nit`,
  result `approve | changes`, `changes` only for an open blocker or should,
  pre-existing defects reported as follow-ups.
- **Review shape**, enforced with a JSON output schema on Codex `turn/start`:
  `{ verdict: approve|changes, summary, findings: [{ id, severity, file,
  line?, claim, fix }], follow_ups[], commands_run: [{ cmd, exit_code }] }`.
- **Coder can disagree.** It answers each finding: fixed, disagree (with
  evidence), or declined. It does not agree just to end the review.

### `delegate_task`

- **Reached** through an in-process SDK MCP server on Claude's stream-json
  control channel. No bridge. ADR 0003 amended.
- **Arguments:** `{ intent, out_of_scope, change_summary, checks, ledger? }`.
  shift adds your original request word for word and the base commit. The
  reviewer is told to flag any mismatch between your request and the coder's
  intent.
- **Blocking.** The call waits for the review. Set `MCP_TOOL_TIMEOUT` high at
  launch. The step 2 spike passed: a stub call that blocked for 10 minutes
  returned its review and Claude continued the turn (Claude Code 2.1.325,
  `MCP_TOOL_TIMEOUT` of 30 minutes), so there is no task id or `task_status`.
- **Rounds:** at most 3, enforced by shift. Stop early when a round returns
  only nits. A finding still disputed after two replies, or a spent budget,
  is **Stuck**: shift returns that to the coder, and the coder pane shows it
  to you with both sides. No extension to 5 rounds in M1.
- **Later rounds** continue the same reviewer session, checking open findings
  and changed code only. The reviewer never sees Claude's history. shift
  stores the ledger so the reviewer pane can show each finding's history.
- **Failures:** if Codex crashes, times out, or returns output that fails the
  schema, `delegate_task` returns a tool error, never a `changes` review. No
  automatic retry, and it does not use a round. Each review has a 15-minute
  wall-clock limit, which also stands in for `max_turns` on Codex.

### Isolation

- **Snapshot:** shift records `HEAD` when the coder session starts as the
  base. On each `delegate_task`, it commits the working tree (tracked changes
  plus untracked, non-ignored files) without touching the coder's branch,
  and checks it out as a temporary worktree. The review refers to that exact
  snapshot. The worktree is removed afterwards.
- **Reviewer sandbox:** Codex `workspace-write` in the worktree, so it can
  build and test without touching your checkout.
- **Build cache:** the reviewer's `CARGO_TARGET_DIR` is a persistent cache
  owned by shift, separate from yours, added to Codex's writable roots. Only
  the first review pays for a cold gpui-kit build.

### Agents and permissions

- **Coder permissions:** a minimal policy gate. shift answers Claude's host
  permission prompts: allow if the tool is in the agent's `tools`, deny
  otherwise. No permission UI.
- **`shell.exec`, not `shell.test`.** The gate cannot tell a test command
  from any other command, so the coder gets unrestricted shell, and the tool
  name says so.
- **Models:** coder uses your usual Claude coding model. Reviewer uses the
  latest Sol through Codex; if Sol is unavailable the run fails rather than
  substituting another model.
- **Reviewer pane is read-only.** You talk to the coder; the coder talks to
  the reviewer.

### Running it

- **Launch:** `shift <path>`, defaulting to the current directory. No repo
  picker.
- **Dogfooding:** run M1 from an installed copy (`cargo install --path
  crates/shift-app`), started from a normal terminal, not from inside a
  Claude Code session (`RLIMIT_RTTIME`). The coder's builds then never touch
  the running app.
- **Clean start:** shift refuses to start a coder session on a dirty working
  tree ("commit or stash first"), so every reviewed diff is the coder's
  alone.
- **Endpoint:** shift creates a `shift/<slug>` branch when the coder session
  starts. The change is left uncommitted there; you review and commit it. No
  PR or merge in M1.
- **During a review** the input is disabled and a **Cancel review** button
  shows. Cancelling returns a "cancelled by user" tool error and does not
  use a round.
- **Transcripts:** raw Claude NDJSON and Codex JSON-RPC are saved to
  `~/.local/state/shift/sessions/<id>/`. They serve as recorded-session test
  fixtures and as evidence for the honest note. Ledger and round state are
  in memory only; nothing resumes after a restart.

## Out of scope for M1

Each of these is deferred, not rejected:

- voice input
- meta-thread intent routing (the coder → reviewer flow is hard-coded)
- Antigravity, Pi, Oh My Pi, and OpenCode
- the flow file format and a general message bus
- the full policy gate

## Reuse from crewroom

Copy and adapt; don't take in its workflow machinery. crewroom is MIT
licensed (Copyright (c) 2026 Crewroom contributors). When you copy or
closely port any of its code, add an entry under "Incorporated code" in
`THIRD_PARTY_NOTICES.md` in the same commit.

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
