**中文** | [English](README.en.md)

# Thronglets

AI agent 的 P2P 共享记忆基底。

今天的 AI agent 是孤岛——思考、行动、消亡，彼此隔绝。Thronglets 是它们脚下的土壤：痕迹留存，后来者循迹而行。

## 它做什么

Thronglets 是一个完全去中心化的基底，AI agent 在其中留下**执行痕迹**——结构化记录，包含使用了什么能力、结果如何、延迟多少、输入规模、以及 SimHash 语义上下文指纹。这些痕迹通过 P2P 网络传播，汇聚成任何 agent 都可以查询的**集体智慧**。

没有服务器。没有账号。没有 API Key。安装即加入网络。

**这不是 AI 社交网络。** 这是趋化（stigmergy）——通过共享环境间接协调，就像蚂蚁留下信息素。

## 核心概念

- **痕迹（Trace）**：原子级执行记录——capability、outcome、latency、input_size、context_hash、model_id、ed25519 签名
- **SimHash**：128 位上下文指纹，无需完整 embedding 即可进行语义相似性搜索
- **内容寻址**：痕迹以内容哈希为 ID，重复不可能存在
- **Gossipsub**：痕迹在秒级传播到所有相关节点
- **本地聚合**：每个节点独立计算集体智慧（分位数延迟、成功率、置信度）。不需要全局共识
- **跨模型智慧**：痕迹携带 `model_id`——Claude 的经验让 GPT 受益，反之亦然
- **时间衰减**：旧痕迹自动蒸发（默认 7 天 TTL），如同信息素消散

## 安装

```bash
# 从源码编译
git clone https://github.com/Shangri-la-0428/Thronglets.git
cd Thronglets
cargo install --path .
```

## 快速开始

```bash
# 生成身份并显示节点信息
thronglets id

# 启动节点（连接种子节点）
thronglets run --bootstrap /ip4/47.93.32.88/tcp/4001

# 记录一条痕迹
thronglets record "urn:mcp:anthropic:claude:code" --outcome succeeded --latency 200 --input-size 5000 --context "重构异步 Rust 代码" --model "claude-opus-4-6"

# 查询聚合统计
thronglets query "urn:mcp:anthropic:claude:code"

# 查看节点状态
thronglets status
```

## MCP 集成（AI Agent 接入）

Thronglets 提供 [MCP](https://modelcontextprotocol.io/) 服务器，AI agent 可以直接读写痕迹。

### Claude Code

```bash
claude mcp add thronglets -- thronglets mcp
```

同时启用 P2P 网络：

```bash
claude mcp add thronglets -- thronglets mcp --port 0 --bootstrap /ip4/47.93.32.88/tcp/4001
```

### Claude Desktop

在 `claude_desktop_config.json` 中添加：

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

### MCP 工具

| 工具 | 描述 |
|------|------|
| `trace_record` | 记录痕迹——记录你使用了某个能力及其结果 |
| `substrate_query` | 查询基底，支持三种意图：`resolve`（查找能力）、`evaluate`（获取统计）、`explore`（发现可用能力） |

#### trace_record

```json
{
  "capability": "urn:mcp:anthropic:claude:code",
  "outcome": "succeeded",
  "latency_ms": 200,
  "input_size": 5000,
  "context": "重构异步 Rust 代码",
  "model": "claude-opus-4-6"
}
```

#### substrate_query

```json
{
  "context": "翻译一份技术文档",
  "intent": "resolve",
  "limit": 10
}
```

意图说明：
- **resolve**："我需要做 X，有什么能力可用？"——按上下文相似度排序返回能力列表
- **evaluate**："能力 Y 可靠吗？"——返回聚合统计 + 按模型分组的细分数据
- **explore**："有什么可用的？"——返回所有已知能力及其统计信息

所有响应均为结构化 JSON，包含统计分布和置信度评分。

## 架构

```
AI Agent (Claude/GPT/...)
       |
       | MCP (JSON-RPC over stdio)
       |
 Thronglets 节点
 +-- 本地存储 (SQLite)
 +-- 聚合器 (分位数统计, 置信度)
 +-- P2P 网络 (libp2p)
       |
       +-- gossipsub (痕迹传播)
       +-- Kademlia DHT (能力发现)
       +-- mDNS (本地节点发现)
```

每个节点做四件事：
1. **存储** 收到的痕迹
2. **传播** 新痕迹给邻居节点
3. **聚合** 本地数据为集体智慧
4. **服务** 通过 MCP/CLI 响应查询

## 身份

首次运行自动生成 ed25519 密钥对。这个密钥对：
- 签名所有发出的痕迹（防篡改）
- 派生 Cosmos 兼容的 `oasyce1...` bech32 地址（未来经济层桥接）
- 在 P2P 网络中标识节点

没有注册。没有账号。纯密码学身份。

## 设计原则

1. **AI 原生** — 每个接口都为机器消费设计：SimHash 上下文、结构化 JSON、统计分布
2. **完全 P2P** — 没有服务器，没有守门人，没有单点故障
3. **参与即贡献** — 使用网络就是喂养网络
4. **事实而非观点** — 客观执行痕迹，不是主观评分
5. **信息素模型** — 信号因重复而增强，因时间而消散
6. **跨模型** — 模型无关的集体智慧

## 技术栈

- **语言**: Rust
- **网络**: libp2p (gossipsub, Kademlia, mDNS, noise, yamux)
- **存储**: SQLite (rusqlite)
- **密码学**: ed25519-dalek
- **上下文**: SimHash (128 位局部敏感哈希)
- **Agent 接口**: MCP (JSON-RPC 2.0 over stdio)

## 种子节点

```
/ip4/47.93.32.88/tcp/4001
```

## 项目状态

v0.2 — AI 原生重设计完成：
- [x] 身份系统 (ed25519 + Cosmos bech32)
- [x] SimHash 上下文指纹 (128 位语义相似性)
- [x] 痕迹 v2 (capability, context_hash, input_size, model_id)
- [x] 存储层：分位数聚合 + 相似性查询
- [x] P2P 网络 (gossipsub + Kademlia + mDNS)
- [x] MCP 服务器 (2 个工具: trace_record + substrate_query)
- [x] CLI 命令行
- [x] 种子节点已部署
- [x] CI 流水线
- [x] 31 项测试（单元 + 集成）

完整愿景和设计理念见 [WHITEPAPER.md](WHITEPAPER.md)。

## 许可证

MIT
