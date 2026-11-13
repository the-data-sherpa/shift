# Context: shift vocabulary

Use these terms consistently in code, docs, and conversation.

**Harness**: The whole runtime. It owns input (voice and text), the meta
thread, the bus, agent lifecycles, and enforcement of capabilities.

**Meta thread**: The conversation the user has with the harness. It
classifies intent, picks a flow, starts it, and reports results back. It does
no domain work itself.

**Flow**: A named composition of roles for one kind of intent. It defines
which roles take part, how work moves between them, and when the flow ends.
Examples: `research`, `build`, `bugfix`, `document`, `decide`.

**Role**: A slot in a flow, such as `scout` or `validator`. A flow is written
in terms of roles. Each role is filled by an agent.

**Agent**: A narrowly scoped worker. It has a model, a system prompt, an
allowlist of skills and tools, and limits such as max turns. Each agent runs
in its own isolated thread. This matches a Standard Agent Spec agent.

**Skill**: A reusable capability that an agent may call, such as `debug`,
`code-read`, `deep-research`, or `grilling`. An agent's definition lists the
skills it is allowed to use. The harness rejects calls to any other skill.

**Tool**: A low-level action, such as reading a file, writing a file, running
a command, or searching the web. Skills are built on tools. Tool access is
where hard capability limits get enforced.

**Bus**: The message channel shared by the agents in a flow. Messages are
typed, for example `finding`, `request`, `handoff`, `verdict`, and `artifact`.
The meta thread and the UI subscribe to it.

**Backend harness**: An existing agent harness that shift launches and
drives, such as Codex, Claude Code, Antigravity, Pi, Oh My Pi, or OpenCode.
shift does not call model APIs directly (see ADR 0001).

**Adapter**: shift's integration with one backend harness. It compiles an
agent definition into that harness's launch config and drives the session.
It also reports the harness's capabilities.

**Policy gate**: The single check in `shift-core` that every adapter sends
its harness's approval requests to. It allows or denies each action based on
the agent definition (see ADR 0003).

**Instance**: One running session of a backend harness, configured as one
agent.

**shift MCP server**: The MCP endpoint injected into every instance, with a
token scoped to that session. It exposes `delegate_task`, bus tools, and
handoff tools. It is how agents reach the bus.

**Delegation**: An agent asks shift to run a subtask as a new child instance,
possibly on a different backend harness. The child gets only the task and its
role, not the parent's history. Its privileges can only be equal to the
parent's or narrower.

**Capability boundary**: The rule that an agent cannot act outside its role.
The harness enforces it through tool and skill allowlists, not through prompt
wording.
