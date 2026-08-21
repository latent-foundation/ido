---
title: Ship the semantic index
status: in-progress
priority: high
tags: mcp, search
order: 0
due: 2026-09-15
goal: p2-semantic-search
---

Hybrid retrieval: keyword hits and embedding hits, fused with RRF so neither
side can dominate a query it happens to be bad at.

- [x] pick the embedding crate
- [ ] build the index on well open
- [ ] wire the `mode` param on `search`
- [ ] eval harness

Design: [[mcp-server]].
