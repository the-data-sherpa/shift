---
name: bugfix
intent: The user reports something broken and wants it fixed.
roles:
  scout: scout
  coder: coder
  reviewer: reviewer
handoffs:
  - scout -> coder        # on: finding
  - coder -> reviewer     # on: artifact (diff)
  - reviewer -> coder     # on: review=changes
exit:
  - reviewer review=approve
  - max_rounds: 3 (then report Stuck to the meta thread)
---

Example flow definition. Roles map to agents in `agents/`. Handoffs are bus
message types that move work from one role to the next.
