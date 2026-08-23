# zfs

ZFS is the filesystem and volume manager backing storage on both Proxmox nodes. Chosen mainly for
checksummed data (silent corruption gets caught and, with redundancy, repaired automatically) and
for how cheap it makes snapshots — a snapshot is close to instant and costs almost nothing until the
underlying blocks actually diverge.

The pool backing VM disks is named `tank-zpool01`. See [[Proxmox]] for how it's used in the cluster,
and [[homelab-glossary]] for adjacent terms.
