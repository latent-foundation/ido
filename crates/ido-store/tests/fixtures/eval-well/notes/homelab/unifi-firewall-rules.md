# UniFi firewall rules

Running UniFi Network 8.1.113 on the Cloud Gateway. These are the rules I actually want, written
down before I forget the reasoning behind each one.

## Rule order matters

UniFi evaluates top to bottom and stops at the first match, so the more specific rules (block IoT
from reaching the trusted VLAN) have to sit above the general "allow established" catch-all, or
they never fire.

## Rules

1. Allow established/related — always first
2. Block IoT (VLAN 30) → Trusted (VLAN 10), any port
3. Allow Trusted → Servers (VLAN 20), any port
4. Block Guest (VLAN 42) → everything except WAN
5. Allow Servers → WAN on 443/80 only (updates), block everything else outbound from that VLAN

## Logging

Rule 2 has logging turned on temporarily — I want to see if anything on the IoT VLAN is actually
trying to phone the trusted network before I'm confident the rule is safe to leave silent.

## Gotcha

The Cloud Gateway silently drops rules referencing a VLAN that was deleted and recreated with the
same name — it keeps the old internal ID. Recreating a rule from scratch fixed a rule that looked
correct in the UI but never matched anything.
