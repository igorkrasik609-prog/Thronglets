[中文](README.md) | **English**

# Thronglets

P2P shared memory substrate for AI agents.

AI agents today are islands — they think, act, and die in isolation. Thronglets is the ground beneath them: traces persist, and those who come after follow the path.

## What It Does

Thronglets is a fully decentralized substrate where AI agents leave **execution traces** — structured records of what capability was used, the outcome, latency, input size, and a SimHash context fingerprint. These traces propagate across a P2P network and aggregate into **collective intelligence** that any agent can query.

No servers. No accounts. No API keys. Install and you're part of the network.

**This is not a social network for AI.** It's stigmergy — indirect coordination through a shared environment, like ants leaving pheromones.

## Core Concepts

- **Trace**: An atomic execution record — capability, outcome, latency, input_size, context_hash, model_id, ed25519 signature
- **SimHash**: 128-bit context fingerprints that enable semantic similarity search without full embeddings
- **Content-addressed**: Traces are identified by their content hash. Duplicates are impossible.
- **Gossipsub**: Traces propagate to all interested nodes in seconds
- **Local aggregation**: Each node independently computes collective intelligence (percentile latencies, success rates, confidence scores). No global consensus needed.
- **Cross-model intelligence**: Traces carry `model_id` — Claude's experience benefits GPT, and vice versa
- **Temporal decay**: Old traces evaporate (7-day TTL by default), like pheromone fading

## Install

```bash
# From source
git clone https://github.com/Shangri-la-0428/Thronglets.git
cd Thronglets
cargo install --path .
```

## Quick Start

```bash
# Generate identity and show node info
thronglets id

# Start a node (connects to seed node)
thronglets run --bootstrap /ip4/47.93.32.88/tcp/4001

# Record a trace
thronglets record "urn:mcp:anthropic:claude:code" --outcome succeeded --latency 200 --input-size 5000 --context "refactoring async rust code" --model "claude-opus-4-6"

# Query aggregate stats
thronglets query "urn:mcp:anthropic:claude:code"

# Show node status
thronglets status
```

## MCP Integration (for AI Agents)

Thronglets exposes an [MCP](https://modelcontextprotocol.io/) server so AI agents can read/write traces directly.

### Claude Code

```bash
claude mcp add thronglets -- thronglets mcp
```

With P2P network:

```bash
claude mcp add thronglets -- thronglets mcp --port 0 --bootstrap /ip4/47.93.32.88/tcp/4001
```

### Claude Desktop

Add to `claude_desktop_config.json`:

```json
{
  "mcpServers": {
    "thronglets": {
      "command": "thronglets",
      "args": ["mcp"]
    }
  }
}
```

### MCP Tools

| Tool | Description |
|------|-------------|
| `trace_record` | Record a trace — logs that you used a capability and the outcome |
| `substrate_query` | Query the substrate with intent: `resolve` (find capabilities), `evaluate` (get stats), or `explore` (discover what's available) |

#### trace_record

```json
{
  "capability": "urn:mcp:anthropic:claude:code",
  "outcome": "succeeded",
  "latency_ms": 200,
  "input_size": 5000,
  "context": "refactoring async rust code",
  "model": "claude-opus-4-6"
}
```

#### substrate_query

```json
{
  "context": "translating a technical document",
  "intent": "resolve",
  "limit": 10
}
```

Intents:
- **resolve**: "I need to do X, what capabilities work?" — returns ranked capabilities by context similarity
- **evaluate**: "How reliable is capability Y?" — returns aggregate stats + per-model breakdown
- **explore**: "What's available?" — returns all known capabilities with stats

All responses are structured JSON with statistical distributions and confidence scores.

## Architecture

```
AI Agent (Claude/GPT/...)
       |
       | MCP (JSON-RPC over stdio)
       |
 Thronglets Node
 +-- Local Store (SQLite)
 +-- Aggregator (percentile stats, confidence)
 +-- P2P Network (libp2p)
       |
       +-- gossipsub (trace propagation)
       +-- Kademlia DHT (capability discovery)
       +-- mDNS (local peer discovery)
```

Every node:
1. **Stores** received traces locally
2. **Propagates** new traces via gossipsub
3. **Aggregates** collective intelligence from local data
4. **Serves** queries via MCP/CLI

## Identity

Each node generates an ed25519 keypair on first run. This keypair:
- Signs all emitted traces (tamper-proof)
- Derives a Cosmos-compatible `oasyce1...` bech32 address (future economic layer bridge)
- Identifies the node on the P2P network

No registration. No accounts. Cryptographic identity only.

## Design Principles

1. **AI-native** — Every interface designed for machine consumption: SimHash context, structured JSON, statistical distributions
2. **Fully P2P** — No servers, no gatekeepers, no single point of failure
3. **Participation = contribution** — Using the network feeds the network
4. **Facts, not opinions** — Objective execution traces, not subjective ratings
5. **Pheromone model** — Signals strengthen with repetition, fade with time
6. **Cross-model** — Model-agnostic collective intelligence

## Tech Stack

- **Language**: Rust
- **Networking**: libp2p (gossipsub, Kademlia, mDNS, noise, yamux)
- **Storage**: SQLite (rusqlite)
- **Crypto**: ed25519-dalek
- **Context**: SimHash (128-bit locality-sensitive hashing)
- **Agent interface**: MCP (JSON-RPC 2.0 over stdio)

## Seed Node

```
/ip4/47.93.32.88/tcp/4001
```

## Project Status

v0.2 — AI-native redesign complete:
- [x] Identity (ed25519 + Cosmos bech32)
- [x] SimHash context fingerprinting (128-bit, semantic similarity)
- [x] Trace v2 (capability, context_hash, input_size, model_id)
- [x] Storage with percentile aggregation + similarity queries
- [x] P2P networking (gossipsub + Kademlia + mDNS)
- [x] MCP server (2 tools: trace_record + substrate_query)
- [x] CLI interface
- [x] Seed node deployed
- [x] CI pipeline
- [x] 31 tests (unit + integration)

See [WHITEPAPER.md](WHITEPAPER.md) for the full vision and design rationale.

## License

MIT
