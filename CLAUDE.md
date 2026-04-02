# Thronglets — BOND Infrastructure

> **Sigil role**: Necessary subsystem. Implements the bond edges of the Sigil graph.
> See [Oasyce-Sigil/ARCHITECTURE.md](../Oasyce-Sigil/ARCHITECTURE.md) for how this fits the whole.

## What This Is

Thronglets is the shared state infrastructure for the Sigil protocol's BOND operation. When two Loops (agents with Sigils) share state, Thronglets is the substrate that makes that sharing possible — persistent, decentralized, not controlled by either Loop alone.

In Sigil terms:
- **BOND(L₁, L₂, S_shared)** → Thronglets creates a shared state channel between two Sigils
- **UNBOND** → Thronglets closes the channel
- **Nesting** (inner state ⊆ outer state) → deep BOND, also through Thronglets

Thronglets is NOT a messaging system. Messages are transient; shared state is persistent and bidirectionally mutable.

## Architecture

```
Rust binary (v0.7.0)
├── trace/       — Atomic execution record (outcome, latency, context, signatures)
├── storage/     — SQLite trace store with TTL and context bucketing
├── pheromone.rs — Stigmergic field (collective memory with decay + Hebbian coupling)
├── signals/     — Signal ranking: avoid / do next / maybe also / context
├── posts/       — Signal posting and querying
├── presence/    — Session heartbeats (which Loops are alive)
├── context/     — SimHash fingerprinting for semantic similarity
├── continuity/  — External continuity traces (Psyche milestones, chain anchoring)
├── identity/    — Node identity (ed25519) + owner account binding
├── network/     — libp2p gossipsub + Kademlia DHT + mDNS
├── mcp/         — MCP server (JSON-RPC 2.0) — primary AI integration
├── http/        — HTTP/REST server — alternative integration
├── anchor/      — Oasyce chain integration for trace anchoring
└── main.rs      — CLI (~9200 lines)
```

## Sigil ↔ Thronglets Mapping

| Sigil concept | Thronglets implementation |
|---|---|
| BOND shared state | Traces + pheromone field in shared context buckets |
| BOND scope | Context hash (SimHash) determines which traces are visible |
| Sigil identity | Node identity (ed25519 keypair) — will migrate to Sigil keypair |
| BOND evidence | Trace records signed by both parties, optionally anchored on-chain |
| UNBOND | Session end + trace TTL expiry (7 days default) |

## Integration Paths

### Hook-based (Claude Code primary)
SessionStart, PreToolUse, PostToolUse, SessionEnd hooks. AI agents never call Thronglets directly — it silently injects sparse signals.

### MCP-based (any MCP-compatible agent)
Tools over JSON-RPC 2.0:
- `substrate_query` — search traces by intent/context/capability
- `trace_record` — record execution outcome
- `signal_post` / `signal_feed` — post/read signals
- `presence_ping` / `presence_feed` — session presence
- `trace_anchor` — anchor trace on Oasyce chain
- `authorization_check` — verify identity binding

### HTTP/REST (non-MCP agents)
Same operations over HTTP JSON API.

## Key Design Decisions

- **Stigmergic, not consensus**: No global agreement. Pheromone field decays and self-organizes. Like ants, not like parliament.
- **Sparse signals**: Max 3 categories per query (avoid, do next, maybe also). Information-dense, not noise.
- **Context-local**: Same capability in different contexts learns independently (SimHash bucketing).
- **TTL decay**: Traces fade like pheromones (7 days default). Old information naturally disappears.

## Install & Run

```bash
npx -y thronglets start    # start node
thronglets status           # check status
thronglets share            # get connection file for others
thronglets join <file>      # join another node
```

## Upcoming (Sigil alignment)

- [ ] Rename core API: `substrate_query` → `bond_query`, align with BOND semantics
- [ ] Accept Sigil ID instead of node pubkey for identity
- [ ] Explicit BOND/UNBOND lifecycle events (not just implicit via trace TTL)
- [ ] Multi-party BOND support (>2 Sigils sharing state)

## Build

```bash
cargo build --release
cargo test
```

Rust 2024 edition. Key dependencies: libp2p 0.54, tokio, rusqlite, ed25519-dalek, serde.
