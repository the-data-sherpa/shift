# Open questions

Not decided yet. Resolve each one before building the part it affects.

## Runtime
- ~~Own the agent loop or orchestrate harnesses?~~ We orchestrate existing
  harnesses. See `adr/0001`.
- ~~Language~~: Rust with GPUI. See `adr/0002`.
- ~~First adapters~~: Claude Code, Codex, and Antigravity. See `adr/0002`.
  Pi, Oh My Pi, and OpenCode come later.
- **Agent `harness` field.** Is it fixed per agent, or can the flow or the
  delegating agent override it? t3code lets the caller choose a target.
- ~~Adapter strategy~~: native protocol per harness, a shared policy gate, and an
  MCP stdio bridge. See `adr/0003`.
- **Boundary gaps.** When a harness can't enforce a role's limits, should
  shift refuse to launch that role there, or fall back to a sandbox it
  controls?
- **Native subagents.** Should a harness's own subagent feature be allowed
  for same-provider work, the way t3code allows it, or should everything go
  through `delegate_task` so the bus sees it all?
- **Standard Agent Spec.** Should we be spec-compliant, or only borrow its
  ideas? Being compliant means we could install other people's agents, but it
  ties the agent format to the spec's (code-defined) shape.
- **Routing.** Does the meta thread pick a flow with an LLM classifier, rules,
  or both? What happens when the intent is ambiguous: ask the user, or run
  `research` first?

## Bus
- In-process channels to start, or a real broker (NATS, Redis streams) from
  day one?
- Can agents message any role, or only along the flow's declared handoffs?
- Is bus history the source of truth for replay and debugging (event-sourced)?

## Capability boundaries
- Where are limits enforced: tool dispatch in the harness, OS sandboxing, or
  both?
- How does an agent escalate when a task is outside its role (for example, a
  coder needs a design decision)?

## Skills
- Should skills use the existing `SKILL.md` format so we can reuse current
  skills?
- Can more than one agent use the same skill?

## Voice
- Speech-to-text and text-to-speech: local (whisper.cpp, piper) or hosted?
- Push-to-talk or a wake word? Can the user interrupt a running flow?

## UI
- What does the main view show: the meta thread, a flow graph, per-agent
  threads, the bus log?
- Should the first UI be a TUI or CLI to prove the core before building GPUI?
