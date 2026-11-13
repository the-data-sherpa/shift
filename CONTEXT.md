# Context: shift vocabulary

shift orchestrates existing agent harnesses so narrow agents, each on the
harness that suits its role, can work together. Use these terms consistently
in code, docs, and conversation.

## Runtime

**Orchestrator**: shift's own runtime. It owns input (voice and text), the
meta thread, the bus, session lifecycles, and enforcement of capabilities.
_Avoid_: harness (for shift itself), runtime

**Harness**: An existing agent tool that shift launches and drives, such as
Codex, Claude Code, Antigravity, Pi, Oh My Pi, or OpenCode. shift does not
call model APIs directly (see ADR 0001).
_Avoid_: backend harness, provider

**Adapter**: shift's integration with one harness. It compiles an agent
definition into that harness's launch config and drives the session. It also
reports the harness's capabilities.

**Session**: One agent's conversation inside one harness. It survives a
restart of the harness process (for example, a resume).
_Avoid_: instance, thread (Codex's `thread` is an adapter detail)

**Policy gate**: The single check that answers every approval request from
every harness. It allows or denies each action based on the agent definition
(see ADR 0003).

**shift MCP server**: The MCP endpoint injected into every session, scoped
to that session. It exposes `delegate_task`, bus tools, and handoff tools. It
is how agents reach the orchestrator.

## Work

**Meta thread**: The conversation the user has with the orchestrator. It
classifies intent, picks a flow, starts it, and reports results back. It does
no domain work itself.

**Flow**: A named composition of roles for one kind of intent. It defines
which roles take part, how work moves between them, and when the flow ends.
Examples: `research`, `build`, `bugfix`, `document`, `decide`.

**Role**: A slot in a flow, such as `scout` or `validator`. A flow is written
in terms of roles. Each role is filled by an agent.

**Agent**: A narrowly scoped worker. It has a model, a system prompt, an
allowlist of skills and tools, and limits such as max turns. Each agent runs
in its own session. This matches a Standard Agent Spec agent.

**Skill**: A reusable capability that an agent may call, such as `debug`,
`code-read`, `deep-research`, or `grilling`. An agent's definition lists the
skills it is allowed to use. The orchestrator rejects calls to any other
skill.

**Tool**: A low-level action, such as reading a file, writing a file, running
a command, or searching the web. Skills are built on tools. Tool access is
where hard capability limits get enforced.

**Bus**: The message channel shared by the agents in a flow. Messages are
typed, for example `finding`, `request`, `handoff`, `verdict`, and `artifact`.
The meta thread and the UI subscribe to it.

**Delegation**: An agent asks the orchestrator to run a subtask in a new
child session, possibly on a different harness. The child gets only the task
and its role, not the parent's history. Its privileges can only be equal to
the parent's or narrower.

**Review**: A reviewer's structured judgement of one change against the
user's request: `approve` or `changes`, with findings and the commands it
ran. `changes` means at least one blocker or should finding is open.
_Avoid_: verdict, pass/fail (for code review)

**Finding**: One problem a review reports, with a stable id and a severity:
`blocker` (the requested behavior fails, or a reachable safety violation),
`should` (another reachable regression the change introduced), or `nit`.

**Follow-up**: A pre-existing defect a review notices but that the change did
not introduce. It is out of scope for the change.

**Round**: One review of the current change. Later rounds look only at open
findings and code changed since the last round.

**Ledger**: The coder's answer to each finding across rounds: fixed,
disagreed (with evidence), or declined.

**Stuck**: The end state when a finding stays disputed after two replies or
the round budget runs out. It goes to the user, never counts as approval.

**Capability boundary**: The rule that an agent cannot act outside its role.
The orchestrator enforces it through tool and skill allowlists, not through
prompt wording.
