---
name: bugfix
intent: The user reports something broken and wants it fixed.
roles:
  scout: scout
  coder: coder
  validator: validator
handoffs:
  - scout -> coder        # on: finding
  - coder -> validator    # on: artifact (diff)
  - validator -> coder    # on: verdict=fail
exit:
  - validator verdict=pass
  - max_cycles: 3 (then report to the meta thread)
---

Example flow definition. Roles map to agents in `agents/`. Handoffs are bus
message types that move work from one role to the next.
