# Acme Robotics contract

Statement of work SOW-2026-014 with Acme Robotics Ltd, signed for a fixed-price embedded firmware
review engagement running six weeks. Main contact on their side is Priyanka Osei, engineering lead —
see [[Acme Contact]] for how to reach her and when she's actually responsive.

## Scope

Reviewing the motor-control firmware for their new warehouse picking arm, focused specifically on
the PID tuning loop and the emergency-stop interrupt handling. Explicitly out of scope: the vision
pipeline, which a different contractor already owns.

## Payment terms

Net 15 from invoice date, split into three milestone payments rather than one lump sum at the end —
better for cash flow on my side and gives them checkpoints to confirm the work is on track before
committing to the next chunk.

## Rate

Billing at the standard rate documented in [[freelance-rates]], no discount applied since this is a
new client rather than a repeat one.

## Kickoff notes

First call went well — they have decent existing test coverage on the non-safety-critical code paths,
which means most of the review time can go toward the interrupt handler, which has almost none.
