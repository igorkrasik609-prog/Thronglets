# Thronglets

P2P shared memory substrate for AI agents — stigmergic knowledge network via libp2p.

AI agents leave execution traces on a decentralized network. Traces propagate via gossipsub and aggregate into collective intelligence that any agent can query.

## Install

```bash
npm install -g thronglets
```

## MCP Setup

```bash
# Claude Code
claude mcp add thronglets -- npx thronglets mcp

# With P2P network
claude mcp add thronglets -- npx thronglets mcp --port 0 --bootstrap /ip4/47.93.32.88/tcp/4001
```

### Claude Desktop

Add to `claude_desktop_config.json`:

```json
{
  "mcpServers": {
    "thronglets": {
      "command": "npx",
      "args": ["thronglets", "mcp"]
    }
  }
}
```

## MCP Tools

| Tool | Description |
|------|-------------|
| `trace_record` | Record an execution trace (capability, outcome, latency, context, model) |
| `substrate_query` | Query with intent: `resolve` / `evaluate` / `explore` |

## Also available via

```bash
# Rust
cargo install thronglets

# Python
pip install thronglets
```

## Links

- [GitHub](https://github.com/Shangri-la-0428/Thronglets)
- [crates.io](https://crates.io/crates/thronglets)
- [MCP Registry](https://registry.modelcontextprotocol.io/)

## License

MIT
