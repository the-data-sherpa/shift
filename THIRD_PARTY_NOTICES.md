# Third-party notices and acknowledgements

shift's own code is licensed under the [MIT License](LICENSE). This file
credits the projects shift learned from and lists third-party material that
shift uses or includes. Each third-party component keeps its own license.

## Design references (no code copied)

shift's design draws on the projects below. We studied their public
documentation and source to learn design patterns. **No code or
documentation text from them is included in this repository** unless it is
listed under "Incorporated code" below.

- **t3code** by T3 Tools Inc. (<https://github.com/pingdotgg/t3code>),
  MIT License, Copyright (c) 2026 T3 Tools Inc.
  - We drew on it for the design of the app-owned orchestration MCP server,
    `delegate_task` child threads, session-scoped credentials, the rule that
    a child's privileges can only narrow, and capability-based provider
    adapters. See ADR 0001.
- **Standard Agent Specification** (<https://standardagentspec.org/>,
  <https://github.com/standardagents/spec>)
  - We drew on it for the principles of narrow, domain-specific, composable
    agents with isolated context.
  - We only link to and summarize its ideas. Its repository publishes no
    license, so **do not copy its text into this repository**.
- **crewroom** (<https://github.com/the-data-sherpa/crewroom>), MIT License,
  Copyright (c) 2026 Crewroom contributors.
  - It is the predecessor project, and it informs shift's harness adapter
    approach and lessons learned. See the M1 milestone.

## Incorporated code

None yet.

When code is copied or closely ported from another project, add an entry
here **in the same change**. Each entry needs:

- the source repository and file paths
- the upstream copyright line and license name
- the full license text, if that license requires it to accompany copies
  (MIT does)

Code written independently from a project's documentation or general design
is a design reference. It belongs in the section above.

## Dependencies

Rust dependencies are fetched by Cargo and keep their own licenses. Key UI
dependencies:

- **GPUI Kit** (`gpui-kit`, `gpui-component`, `gpui-base`, `gpui-kit-assets`)
  by Longbridge (<https://github.com/longbridge/gpui-kit>): Apache-2.0
- **GPUI**, published as `gpui-pre`, a snapshot of Zed's GPUI
  (<https://github.com/zed-industries/zed>): Apache-2.0

Before distributing shift binaries, generate a full dependency license
bundle (for example with `cargo about`) and ship it with the binary.
Apache-2.0 and MIT both require that their notices accompany redistributed
copies.

## Trademarks

Claude and Claude Code (Anthropic), Codex (OpenAI), Antigravity (Google), T3
and t3code (T3 Tools Inc.), Zed (Zed Industries), and other product names are
trademarks of their respective owners. They are used only to identify the
software shift works with or learned from. shift is an independent project.
It is not affiliated with or endorsed by any of these companies.
