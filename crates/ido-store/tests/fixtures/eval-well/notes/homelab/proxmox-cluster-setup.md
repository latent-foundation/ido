# Proxmox cluster notes

Standing up a two-node Proxmox VE cluster: the existing tower gets renamed `pve-node07` and the
incoming 2U rack server becomes `pve-node08`. Quorum needs a third voter, so I'm running a QDevice
on the Raspberry Pi that already sits on the shelf doing DNS.

## Storage plan

Both nodes present a ZFS mirror as local storage; shared VM disks live on a pool named
`tank-zpool01`, replicated between nodes every 15 minutes rather than true shared storage — good
enough for a homelab, and it means no separate SAN box to buy or maintain.

## Networking

Each node gets a dedicated corosync link on its own physical NIC, separate from the VM bridge, so
cluster heartbeat traffic never contends with anything a guest is doing. Corosync is picky about
latency spikes and will start flapping the cluster if that link gets busy.

## Migration order

1. Spin up `pve-node08` fresh, join the cluster
2. Move the DNS and reverse-proxy VMs first (low risk, easy rollback)
3. Move the media server last, since it has the biggest disks and the longest live-migration time

## Open question

Still deciding whether the old NAS becomes a third ZFS replication target or just gets decommissioned
once both nodes have redundant storage. Leaning toward decommissioning — one less box drawing power.
