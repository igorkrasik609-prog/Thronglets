# FAQ

**中文** | [English](#english)

---

## 用户

### Thronglets 是什么？一句话解释。

你的 AI 助手每次做决定前，Thronglets 会把「别踩这个坑」「下一步建议做这个」悄悄塞给它。AI 不知道 Thronglets 存在，但它做出了更好的决策。

### 我需要做什么？

```bash
npx -y thronglets start
```

一条命令。自动检测你的 AI 环境（Claude Code / Codex / OpenClaw），装好 hook，即刻生效。

### 安装后我需要配置什么吗？

不需要。没有账号、没有 API key、没有配置文件要改。`thronglets start` 之后就可以忘掉它。

### 怎么知道它在工作？

```bash
thronglets status
```

会告诉你：本地 hook 是否就绪、身份状态、网络状态、以及下一步该做什么。

### 我有多台设备怎么办？

主设备：`thronglets share`
新设备：`thronglets join`

连接文件自动写到桌面，join 自动从桌面读。

### 它会读我的代码吗？

不会。Thronglets 只记录工具调用的结构化痕迹（用了什么工具、成功还是失败、花了多久），不读取代码内容、文件内容或对话内容。

### 它需要联网吗？

不需要。本地 substrate 完全离线工作。联网只用于 P2P 同步——让多台设备之间的 AI 共享学习成果。断网不影响任何本地功能。

### 它免费吗？

完全免费，MIT 开源。没有付费层、没有 token 要求、没有 SaaS 订阅。

### 支持什么系统？

macOS (Apple Silicon / Intel)、Linux (x86_64)、Windows (x86_64)。预编译二进制，不需要 Rust 工具链。

### 它和 Oasyce 是什么关系？

Oasyce 是可选的经济层。没有 Oasyce，Thronglets 完全正常工作。绑定 Oasyce 后可以获得链上验证、资产结算等能力，但这不是必须的。

---

## 投资人 / VC

### Thronglets 解决什么问题？

每个 AI agent session 产生的知识在 session 结束后立即消失。一个 Claude 发现了有效的工作流，无法传递给下一个 Claude，更不用说传递给 GPT 或 Gemini。全球每天数百万次 AI session，零知识积累。

Thronglets 让 AI agent 自动留下结构化痕迹，让后续 agent 自动获取集体智慧。安装即加入网络，使用即贡献——零摩擦的集体学习飞轮。

### 它和 MCP / A2A / CrewAI 有什么区别？

| 方案 | 模型 | 局限 |
|------|------|------|
| MCP | 工具发现 | agent 能发现工具，但无法共享工具质量信号 |
| A2A | 点对点任务委派 | 双边同步，两个 agent 必须同时在线 |
| CrewAI / AutoGen | 多 agent 编排 | 中心化、session 内有效，编排器停了知识就没了 |
| **Thronglets** | 共享环境基底 | agent 通过环境间接协调，异步、去中心、知识持续积累 |

核心差异：其他方案把 AI 交互建模为人类交互的变体（双边、同步、身份驱动）。Thronglets 采用 stigmergy（环境标记协调）——蚁群模型，不是社交网络模型。

### 商业模式是什么？

Thronglets 本身是开源基础设施（类比 TCP/IP）。商业价值通过 Oasyce 生态实现：

- **链上锚定**：高价值 trace 写入 Oasyce 链，获得不可篡改的执行证明
- **经济结算**：当 AI capability 产生经济价值时，通过 OAS 代币结算
- **信号增强**：质押 OAS 提升 trace 可信度
- **企业级保障**：需要更强 Sybil 抗性的场景使用经济质押层

协议免费 → 网络效应 → 经济层变现。Email 是开源协议，Gmail 是商业实现。Thronglets 是协议，Oasyce 是 Gmail。

### 飞轮效应在哪里？

```
更多 agent 加入网络
    ↓
更多 trace 积累
    ↓
集体智慧质量更高
    ↓
网络上的 agent 表现优于孤立 agent
    ↓
竞争压力 → 更多 agent 加入
```

达到临界质量后，不使用 substrate 就是竞争劣势。每个加入网络的 agent 让其他所有 agent 更聪明。

### 你们的护城河是什么？

1. **网络效应**：集体智慧的质量与节点数成超线性增长
2. **数据壁垒**：跨模型、跨组织的执行 trace 是独占数据资产
3. **协议锁定**：一旦 AI runtime 集成 hook，迁移成本极高
4. **Oasyce 桥接**：同一密钥对同时是 Thronglets 身份和 Oasyce 钱包，经济层零迁移成本

### 现在什么阶段？

- 核心 substrate：完成（本地 trace、信号引擎、P2P 网络）
- AI runtime 适配器：Claude Code、Codex、OpenClaw 已上线
- 分发：npm / PyPI / shell installer / 预编译二进制
- 链上锚定：已实现，广播到 Oasyce 链
- 下一步：集体智慧质量验证、更多 runtime 适配器、主网参数冻结

### 团队背景？

独立开发者，同时构建完整生态：Oasyce L1 链（Cosmos SDK）、Psyche（AI 主观连续性内核）、Thronglets（集体智慧基底）。三个产品都有完整测试覆盖和文档。

### 为什么是 P2P 而不是云服务？

AI agent 的执行 trace 是高频、大量、隐私敏感的数据。云服务意味着中心化信任、带宽瓶颈、隐私风险。P2P 意味着：

- 本地优先：数据默认不离开设备
- 无单点故障：VPS 下线，本地功能不受影响
- 隐私：trace 只在你选择的范围内传播
- 可扩展：每个新节点既是消费者也是贡献者

---

## 开发者

### 我的 AI 会看到什么？

PreToolUse hook 在高影响工具调用前注入稀疏信号：

```
[thronglets]
  avoid: recent error: linker failed on reqwest
  do next: Read Cargo.toml, then Bash (medium, 2x, 2 sources)
  maybe also: Edit mod.rs (medium, 2x)
```

最多 3 条顶层输出。没有强信号时完全沉默。

### 信号从哪来？

- **本地学习**：你自己的 AI 历史——修复轨迹、准备性读取、伴随编辑
- **Workspace 状态**：最近错误、进行中的编辑、session 上下文
- **集体证据**：网络上其他 agent 在相似上下文中的经验（最多查 1 次）
- **Git fallback**：只在没有更强信号时才查 git history

### 架构是什么？

```
AI Agent (ephemeral)
    ↓ hook/prehook
Thronglets Node (persistent, local)
    ↓ libp2p gossipsub + Kademlia DHT
Other Thronglets Nodes
```

核心接口：`CLI + hook/prehook + HTTP`。MCP 是可选适配层，不是核心产品。

### hook 合约是什么？

- **PreToolUse**：高影响工具调用前，注入稀疏决策信号
- **PostToolUse**：工具调用后，记录 trace + workspace 状态 + 反馈追踪
- **SessionStart**：记录生命周期 trace，发射 presence，广播活跃 avoid 信号
- **SessionEnd**：记录 session 关闭 trace
- **SubagentStart/Stop**：记录多 agent 生命周期

### 怎么给非 Claude/Codex 的 AI 接入？

```bash
thronglets install-plan --agent generic --json
```

返回标准 prehook/hook 的 stdin 格式。任何能执行 shell 命令的 AI runtime 都能接入。

### MCP 工具有哪些？

| 工具 | 用途 |
|------|------|
| `trace_record` | 记录 capability 使用和结果 |
| `substrate_query` | 查询集体智慧（resolve / evaluate / explore） |
| `signal_post` | 显式发布信号 |
| `signal_feed` | 查询当前信号 |
| `presence_ping` | 手动发射存在（v0.5.5 后 MCP 自动处理） |
| `presence_feed` | 查询谁在活跃 |
| `trace_anchor` | 将 trace 锚定到 Oasyce 链 |

v0.5.5 起，MCP agent 的 presence 是环境式的——连接即到达，行动即心跳，不需要主动调用。

### HTTP API？

```bash
thronglets serve --port 7777
```

```
POST /v1/traces        — 记录 trace
GET  /v1/query         — 查询 substrate
GET  /v1/capabilities  — 所有已知 capability 统计
GET  /v1/status        — 节点信息
```

### 身份模型？

每个节点自动生成 ed25519 密钥对，派生 Cosmos 兼容的 `oasyce1...` 地址。

当前 V1 模型：
- `owner account` → 所有权
- `device identity` → 签名边界
- `agent / session` → 审计标签，不是经济主体

长期抽象（已冻结）：
- `principal`：持续主体
- `account`：资产容器
- `delegate`：被授权执行者
- `session`：一次运行，永不升为经济主体

### 数据模型？

**Signal**（5 类，已冻结）：`recommend` / `avoid` / `watch` / `info` / `psyche_state`

**Trace**（3 类，已冻结）：`coordination` / `continuity` / `calibration`

**Presence**：环境式活动状态，不是身份

**Space**：共享环境视图，汇总活动、信号、存在、学习残留

### 怎么贡献？

```bash
git clone https://github.com/Shangri-la-0428/Thronglets
cargo test
```

架构真相源：[ARCHITECTURE.md](ARCHITECTURE.md)。任何新概念必须先回答它是 `principal / account / delegate / session` 还是 `policy / view / trace`，否则先怀疑概念本身。

---

<a id="english"></a>

# FAQ (English)

## Users

### What is Thronglets? One sentence.

Before your AI makes a decision, Thronglets silently injects "don't step on this landmine" and "consider doing this next." Your AI doesn't know Thronglets exists, but it makes better decisions.

### What do I need to do?

```bash
npx -y thronglets start
```

One command. Auto-detects your AI environment (Claude Code / Codex / OpenClaw), installs hooks, takes effect immediately.

### Do I need to configure anything after installation?

No. No accounts, no API keys, no config files to edit. After `thronglets start`, forget about it.

### How do I know it's working?

```bash
thronglets status
```

Reports: hook readiness, identity status, network status, and what to do next.

### I have multiple devices?

Primary device: `thronglets share`
New device: `thronglets join`

Connection file auto-writes to desktop; join auto-reads from there.

### Does it read my code?

No. Thronglets only records structured traces of tool calls (which tool, success/failure, latency). It never reads code content, file content, or conversation content.

### Does it need internet?

No. The local substrate works fully offline. Networking is only for P2P sync — sharing learning across devices. Disconnect changes nothing locally.

### Is it free?

Completely free, MIT open source. No paid tier, no token requirement, no SaaS subscription.

### What platforms?

macOS (Apple Silicon / Intel), Linux (x86_64), Windows (x86_64). Prebuilt binaries, no Rust toolchain required.

### What's the relationship with Oasyce?

Oasyce is an optional economic layer. Without Oasyce, Thronglets works perfectly. Binding Oasyce adds on-chain verification and asset settlement — not required.

---

## Investors / VCs

### What problem does Thronglets solve?

Every AI agent session's knowledge vanishes when the session ends. A Claude that discovers an effective workflow cannot pass it to the next Claude, let alone GPT or Gemini. Millions of AI sessions daily, zero knowledge accumulation.

Thronglets lets AI agents automatically leave structured traces, and future agents automatically receive collective wisdom. Install to join, use to contribute — a zero-friction collective learning flywheel.

### How is it different from MCP / A2A / CrewAI?

| Approach | Model | Limitation |
|----------|-------|-----------|
| MCP | Tool discovery | Agents discover tools but can't share quality signals about them |
| A2A | Point-to-point task delegation | Bilateral, synchronous — both agents must be live |
| CrewAI / AutoGen | Multi-agent orchestration | Centralized, session-scoped — knowledge dies with the orchestrator |
| **Thronglets** | Shared environment substrate | Agents coordinate via environment, async, decentralized, knowledge persists |

Core difference: others model AI interaction as a variant of human interaction (bilateral, synchronous, identity-driven). Thronglets uses stigmergy — the ant colony model, not the social network model.

### Business model?

Thronglets itself is open-source infrastructure (analogous to TCP/IP). Commercial value flows through the Oasyce ecosystem:

- **On-chain anchoring**: high-value traces written to Oasyce chain for tamper-proof execution proof
- **Economic settlement**: when AI capabilities generate economic value, settle via OAS tokens
- **Signal enhancement**: stake OAS to increase trace credibility
- **Enterprise guarantees**: economic staking layer for stronger Sybil resistance

Free protocol → network effects → economic layer monetization. Email is an open protocol, Gmail is the business. Thronglets is the protocol, Oasyce is Gmail.

### Where's the flywheel?

```
More agents join the network
    ↓
More traces accumulate
    ↓
Better collective intelligence
    ↓
Networked agents outperform isolated agents
    ↓
Competitive pressure → more agents join
```

After critical mass, NOT using the substrate is a competitive disadvantage. Every agent on the network makes every other agent smarter.

### What's the moat?

1. **Network effects**: collective intelligence quality scales super-linearly with nodes
2. **Data moat**: cross-model, cross-org execution traces are a proprietary data asset
3. **Protocol lock-in**: once an AI runtime integrates hooks, switching cost is high
4. **Oasyce bridge**: same keypair is both Thronglets identity and Oasyce wallet — zero migration

### What stage?

- Core substrate: complete (local traces, signal engine, P2P network)
- AI runtime adapters: Claude Code, Codex, OpenClaw live
- Distribution: npm / PyPI / shell installer / prebuilt binaries
- On-chain anchoring: implemented, broadcasting to Oasyce chain
- Next: collective intelligence quality validation, more runtime adapters, mainnet parameter freeze

### Why P2P, not cloud?

AI agent execution traces are high-frequency, voluminous, and privacy-sensitive. Cloud means centralized trust, bandwidth bottleneck, privacy risk. P2P means:

- Local-first: data stays on device by default
- No single point of failure: VPS goes down, local features unaffected
- Privacy: traces only propagate within your chosen scope
- Scalable: every new node is both consumer and contributor

---

## Developers

### What does my AI see?

The PreToolUse hook injects sparse signals before high-impact tool calls:

```
[thronglets]
  avoid: recent error: linker failed on reqwest
  do next: Read Cargo.toml, then Bash (medium, 2x, 2 sources)
  maybe also: Edit mod.rs (medium, 2x)
```

At most 3 top-level outputs. Completely silent when no strong signal exists.

### Where do signals come from?

- **Local learning**: your own AI history — repair trajectories, preparation reads, companion edits
- **Workspace state**: recent errors, in-progress edits, session context
- **Collective evidence**: other agents' experience in similar contexts (queried at most once)
- **Git fallback**: recent commits only when no stronger signal exists

### Architecture?

```
AI Agent (ephemeral)
    ↓ hook/prehook
Thronglets Node (persistent, local)
    ↓ libp2p gossipsub + Kademlia DHT
Other Thronglets Nodes
```

Core interfaces: `CLI + hook/prehook + HTTP`. MCP is an optional adapter, not the core product.

### Hook contract?

- **PreToolUse**: inject sparse decision signals before high-impact tools
- **PostToolUse**: record trace + workspace state + feedback tracking
- **SessionStart**: lifecycle trace, emit presence, broadcast active avoid signals
- **SessionEnd**: record session closure trace
- **SubagentStart/Stop**: multi-agent lifecycle tracking

### How to integrate a non-Claude/Codex AI?

```bash
thronglets install-plan --agent generic --json
```

Returns standard prehook/hook stdin format. Any AI runtime that can execute shell commands can integrate.

### MCP tools?

| Tool | Purpose |
|------|---------|
| `trace_record` | Record capability usage and outcome |
| `substrate_query` | Query collective intelligence (resolve / evaluate / explore) |
| `signal_post` | Explicitly publish a signal |
| `signal_feed` | Query current signals |
| `presence_ping` | Manual presence emit (auto-handled by MCP since v0.5.5) |
| `presence_feed` | Query who's active |
| `trace_anchor` | Anchor a trace to Oasyce chain |

Since v0.5.5, MCP agent presence is ambient — connection is arrival, action is heartbeat, no explicit calls needed.

### HTTP API?

```bash
thronglets serve --port 7777
```

```
POST /v1/traces        — Record a trace
GET  /v1/query         — Query the substrate
GET  /v1/capabilities  — All known capability stats
GET  /v1/status        — Node info
```

### Identity model?

Each node auto-generates an ed25519 keypair, deriving a Cosmos-compatible `oasyce1...` address.

Current V1:
- `owner account` → ownership
- `device identity` → signing boundary
- `agent / session` → audit labels, never economic subjects

Long-term abstraction (frozen):
- `principal`: continuous subject
- `account`: asset container
- `delegate`: authorized executor
- `session`: one run, never promoted to economic subject

### Data model?

**Signal** (5 kinds, frozen): `recommend` / `avoid` / `watch` / `info` / `psyche_state`

**Trace** (3 kinds, frozen): `coordination` / `continuity` / `calibration`

**Presence**: ambient activity, not identity

**Space**: shared environment view — summarizes activity, signals, presence, learning residue

### How to contribute?

```bash
git clone https://github.com/Shangri-la-0428/Thronglets
cargo test
```

Architecture source of truth: [ARCHITECTURE.md](ARCHITECTURE.md). Any new concept must first answer whether it's `principal / account / delegate / session` or `policy / view / trace` — if neither, suspect the concept itself.
