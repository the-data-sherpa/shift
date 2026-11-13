# shift

A harness that runs a team of narrowly scoped agents. You talk to it by voice
or text. A meta thread reads what you want, picks a **flow**, and brings in
only the agents that flow needs.

Inspired by the [Standard Agent Specification](https://standardagentspec.org/)
and the orchestration design of [t3code](https://github.com/pingdotgg/t3code).
See [acknowledgements](THIRD_PARTY_NOTICES.md). The Standard Agent
Specification's core idea is composition over inheritance. Each agent covers
one domain, gets only the capabilities it needs, and runs in its own isolated
context.

## Core ideas

1. **Narrow agents.** Each agent does one job and cannot do anything else. A
   coder can't make architecture decisions. A reviewer can't edit code. The
   orchestrator enforces this through tool and skill allowlists. Prompts alone
   aren't enough.
2. **Flows, not free-for-alls.** The user's intent picks a flow. A flow is a
   named set of roles and the order they hand off in. Examples:
   - `research`: researcher, validator, counterpoint
   - `build`: product manager, architect
   - `bugfix`: scout, coder, reviewer
   - `document`: tech writer
   - `decide`: SRE, architect, counterpoint
3. **Skills per agent.** Each agent declares the skills it may call. For
   example, the coder gets `debug`, the scout gets `code-read` and
   `deep-research`, and the architect gets `grilling`.
4. **Message bus.** Agents in a flow talk over a shared bus with typed
   messages. They don't hold direct references to each other. The meta
   thread and the UI both watch the bus.
5. **Voice-first, text-equal.** Voice and text feed the meta thread through
   the same input path.
6. **Native UI.** The UI is probably GPUI (Rust). It shows the meta thread,
   the active flow, each agent's thread, and the bus traffic live.

7. **Orchestrate harnesses, don't replace them.** shift launches Codex,
   Claude Code, Antigravity, Pi, and other harnesses, each configured from a
   shift agent definition. It injects its own MCP server so agents can
   delegate across harnesses. See `docs/adr/0001-orchestrate-existing-harnesses.md`.

## Current milestone

**M1: Claude delegates review to Codex.** See
`docs/milestones/m1-claude-delegates-review-to-codex.md`.

## Build

```sh
cargo test -p shift-core
cargo run            # opens the GPUI window (run from repo root)
```

## Layout (early)

- `crates/shift-core`: domain types (agent definitions, harnesses).
- `crates/shift-app`: the desktop app (`shift` binary), built on GPUI Kit.

- `CONTEXT.md`: domain vocabulary. Read this first.
- `agents/`: agent definitions (role, model, skills, tools, limits).
- `flows/`: flow definitions (intent, roles, handoff order, exit conditions).
- `docs/open-questions.md`: decisions not made yet.

## License

MIT. See [LICENSE](LICENSE). Third-party credits, dependency licenses and
trademark notes are in [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
shift is an independent project. It is not affiliated with Anthropic, OpenAI,
Google, T3 Tools, or Zed.
