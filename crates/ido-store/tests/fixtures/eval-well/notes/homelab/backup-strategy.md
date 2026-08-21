# Backup strategy

Three copies, two media, one offsite — the usual rule, finally actually implemented instead of just
known. Nightly `rclone` job pushes the NAS snapshots to a cheap object storage bucket; a monthly
cold copy goes to an external drive that lives at my parents' house.

## The rclone config

Kept the retry tuning here since I always forget the flag names. The comments inside this block are
part of the script's own documentation, not a real outline of this note — none of the `#` lines
below are section headings, and the `- [ ]` lines are shell comments the previous maintainer left,
not an actual checklist.

```bash
#!/usr/bin/env bash
# Nightly offsite sync
# - [ ] TODO: rotate the API key quarterly
# - [ ] TODO: alert on failure via ntfy
set -euo pipefail

# config: retry behavior
max_retry_backoff_ms=45000
retries=6

rclone sync /mnt/tank/backups remote:homelab-backups \
  --retries "$retries" \
  --low-level-retries 10 \
  --backup-dir "remote:homelab-backups-old/$(date +%F)" \
  --log-file /var/log/rclone-nightly.log

# heading-looking comment inside the fence, must never be parsed as structure
# Section: Restore Procedure
echo "sync complete"
```

## Restore testing

Actually restored a full NAS snapshot to a scratch VM last quarter to make sure the backups are more
than a folder of untested files — worth doing on a calendar reminder, not just when something breaks.
