# Thronglets

P2P shared memory substrate for AI agents.

Thronglets silently injects 8 layers of decision context before every AI tool call, and records execution traces after. Your AI never calls Thronglets — it just makes better decisions.

## Install

```bash
cargo install thronglets
thronglets setup
```

That's it. Two hooks are installed automatically:
- **PreToolUse** injects 8 layers of context before every tool call
- **PostToolUse** records every tool call as a signed trace

## The 8 Layers

| # | Layer | What the AI gets |
|---|-------|------------------|
| 1 | Capability stats | Success rate + latency from collective traces |
| 2 | Workflow patterns | What agents usually do next after this tool |
| 3 | Similar context | Other tools used for similar tasks |
| 4 | Workspace memory | Recent files, errors, previous session summary |
| 5 | Git context | Last 5 commits on the file being touched |
| 6 | Co-edit patterns | Files typically modified together |
| 7 | Preparation reads | Files read before previous edits of this file |
| 8 | Edit retention | % of AI edits that were committed vs reverted |

## MCP Tools (optional)

```bash
claude mcp add thronglets -- thronglets mcp
```

| Tool | Description |
|------|-------------|
| `trace_record` | Record an execution trace |
| `substrate_query` | Query collective intelligence (resolve/evaluate/explore) |
| `trace_anchor` | Anchor trace to Oasyce blockchain |

## P2P Network

```bash
thronglets run --bootstrap /ip4/47.93.32.88/tcp/4001
```

Traces propagate via libp2p gossipsub. Each node independently aggregates collective intelligence.

## Links

- [GitHub](https://github.com/Shangri-la-0428/Thronglets)
- [crates.io](https://crates.io/crates/thronglets)

## License

MIT
