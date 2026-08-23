---
title: Renew the code-signing certificate
status: todo
priority: high
tags: release, chores
order: 2
due: 2026-01-15
goal: v1-release
---

Deliberately overdue, and deliberately by a wide margin: this fixture is
committed, so any due date near "now" stops being interesting the moment the
calendar moves past it. A date well in the past is the one kind that stays
true — which is what makes the `daily_review` prompt (and the rail's overdue
badge, and the calendar's overdue chip) demonstrable against `dev-well` at any
point in the future.

The counterpart case — a task due *today* — can't be represented in a static
fixture at all, so `daily_review`'s "due today" section reads empty here. That
is the fixture's limitation, not the prompt's; the integration tests in
`crates/ido-mcp/tests/server.rs` seed their own well with dates computed off
the real clock precisely so both halves get covered.

No `[[link]]` here on purpose: the fixture carries exactly one dead link
(`[[not-a-page]]`, in the notes) so that "a link to a page that doesn't exist"
stays an unambiguous, single test case rather than ambient noise.
