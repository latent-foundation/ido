# VLAN segmentation plan

The flat network has bugged me for a year — the smart plugs and the NAS sit on the same broadcast
domain as my work laptop. Splitting into VLANs before the rack migration so I'm not redoing cabling
twice.

## Proposed VLANs

| VLAN | Purpose | Subnet |
|---|---|---|
| 10 | Trusted (laptops, desktops) | 10.20.10.0/24 |
| 20 | Servers (Proxmox, NAS) | 10.20.20.0/24 |
| 30 | IoT (plugs, bulbs, the robot vacuum) | 10.20.30.0/24 |
| 42 | Guest wifi | 10.20.42.0/24 |

VLAN 42 is deliberately isolated — guests get internet only, no route to anything else, and it's
rate-limited so a house guest's phone update doesn't saturate the upstream link.

## Inter-VLAN routing

The router handles routing between 10 and 20 (I need to reach the NAS from my laptop); VLAN 30 gets
no route to VLAN 10 at all — if a smart bulb firmware is ever compromised it shouldn't be able to
see anything interesting.

## Switch config

Trunk ports carry all four VLANs; access ports get tagged per device based on a spreadsheet I keep
so I don't forget which port feeds which room.

Depends on [[unifi-firewall-rules]] being written before I actually flip this on — no point
segmenting the network if the firewall between segments is wide open.
