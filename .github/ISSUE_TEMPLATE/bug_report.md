---
name: Bug Report
about: Create a report to help us improve Reclaim
title: '[BUG] '
labels: ['bug', 'needs-triage']
assignees: ''
---

## Description
A clear and concise description of what the bug is.

## Environment
- **Operating System**: macOS / Windows / Linux (specify version, e.g. macOS 15.1 Sequoia / Windows 11 23H2)
- **Architecture**: arm64 (Apple Silicon) / x86_64 (Intel/AMD)
- **Filesystem Type**: APFS / NTFS / ext4 / btrfs / FAT32 / exFAT
- **Reclaim Version**: (e.g. `reclaim version` or desktop app version)

## Steps to Reproduce
1. Scan folder containing '...'
2. Proposed duplicate plan '...'
3. Click 'Execute Quarantine' / run `reclaim execute`
4. Error or unexpected behavior observed

## Expected Behavior
A clear and concise description of what you expected to happen.

## Safety & Invariant Check
- Did Reclaim overwrite any files? (Yes / No — should always be No)
- Was a conflict logged in the SQLite action journal? (Yes / No)
- Is the source file intact? (Yes / No)

## Logs / Diagnostic Output
```text
Paste terminal output or journal errors here
```

