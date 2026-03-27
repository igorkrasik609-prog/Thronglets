# Changelog

## v0.3.0 — 2026-03-28

8-layer decision context engine. Thronglets now injects rich context before every AI tool call.

### New Features

- **Workspace persistence** — Cross-session memory tracking recent files, errors, and sessions in `workspace.json`
- **Git history injection** — Recent 5 commits shown before Read/Write/Edit on any file
- **Decision history** — Co-edit patterns and preparation reads surfaced from tool call sequences
- **Result feedback loop** — Tracks if AI edits were committed or reverted; surfaces retention rate
- **Strategy-level traces** — Auto-detects working patterns (analyze-modify, build-fix-cycle, multi-file-refactor, codebase-exploration, code-review, delegated-research) and labels traces
- **llms.txt + website updated** — Documents all 8 context layers, full EN+ZH i18n

### Tests

- 36 unit tests for workspace module (all methods + save/load roundtrip)

### Performance

- Prehook: 50-70ms (8 layers including git subprocess) — 99% headroom on 5s timeout
- PostToolUse hook: <10ms

## v0.2.1 — 2026-03-27

- `thronglets setup` — one-command Claude Code integration
- PreToolUse prehook — capability stats + workflow patterns + similar context
- PostToolUse hook — automatic trace recording
- Context text preservation in traces
- Session tracking for workflow discovery

## v0.2.0 — 2026-03-26

- SimHash context fingerprinting (128-bit)
- Bucket indexing for O(log n) similarity queries
- Cross-model collective intelligence (model_id field)
- Temporal decay (7-day TTL evaporation)

## v0.1.0 — 2026-03-25

- Core trace structure with ed25519 signing
- SQLite storage with content-addressed IDs
- libp2p gossipsub + Kademlia DHT
- MCP server (trace_record, substrate_query, trace_anchor)
- HTTP REST API
- Oasyce blockchain anchoring
