# proxmox

Proxmox VE is the hypervisor running the homelab cluster: two physical nodes, `pve-node07` and
`pve-node08`, with a Raspberry Pi acting as a QDevice for quorum so a two-node cluster can still
reach consensus if one host drops offline.

VM disks live on a [[ZFS|ZFS pool]] replicated between the two nodes on a schedule rather than true
shared storage — good enough for a homelab where a few minutes of replication lag on failover is an
acceptable tradeoff for not buying a SAN.

See [[homelab-glossary]] for the rest of the homelab terms.
