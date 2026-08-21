# Password manager notes

Moved everything to a password manager two years ago and this is just the leftover setup notes,
kept in case the whole vault ever needs restoring from scratch on a new machine.

## Where the recovery key lives

The emergency recovery kit is printed and kept in the fireproof box with the passport, not stored
digitally anywhere — if the vault provider ever has an outage or goes under, that paper copy is the
only way back in, so it can't live only on a device that could fail at the same time.

## Session keys

Session keys for the browser extension are kept in the OS keychain rather than the vault's own
storage, so unlocking the computer itself is enough to keep the extension logged in without typing
the master password on every restart.
