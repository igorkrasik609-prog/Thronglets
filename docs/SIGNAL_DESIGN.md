# Signal Design Specification

Internal design document. Moved from README.md — this is not user-facing documentation.

For architecture overview, see [ARCHITECTURE.md](../ARCHITECTURE.md).

## Psyche 边界

如果 Psyche 存在，它只是一个**可选 producer**，往现有的 `signal / trace` 抽象里投递少量外化事件。

默认原则：

- Psyche 的输出默认先落 `trace`
- 只有在外在协作上真的需要提醒别人时，才降成 `signal`
- Thronglets 不为 Psyche 扩新增 signal 类别

当前冻结映射：

| Psyche event | Thronglets form | 默认 signal? | 默认只本地缓存? | 可升到 Oasyce SDK? |
|---|---|---|---|---|
| `relation-milestone` | `coordination trace` | 仅在影响协作边界时转 `watch / info` | 通常是 | 条件满足时可以 |
| `writeback-calibration` | `calibration trace` | 默认不是 | 是 | 只升 summary |
| `continuity-anchor` | `continuity trace` | 默认不是 | 可先本地 | 可以，最自然的上升对象 |
| `open-loop-anchor` | `coordination trace` | 可降成 `watch` | 先本地 | 持续且有运营后果时可升 |

当前 runtime 规则已经落地：

- Psyche 继续复用现有 `trace_record` / `POST /v1/traces` 写入口，不新增用户命令
- 通过 `external_continuity` 对象写入的 raw trace 会：
  - 严格校验 `provider=thronglets`、`mode=optional`、`version=1`
  - 固定落到 `coordination / continuity / calibration` 三类 taxonomy
  - 默认只在 Thronglets 本地缓存并衰减，不直接 gossip，不直接形成 DHT summary
- 只有满足条件时，raw trace 才会条件性降级成现有 signal：
  - `relation-milestone -> watch / info`
  - `open-loop-anchor -> watch`
  - `continuity-anchor -> info`
  - repeated `writeback-calibration -> avoid`
- `recommend` 不会由 Psyche 直接产生
- `space --json` 现在会给出本地 continuity 摘要、固定 ruleset、每条 trace 当前是 `local-only / derived-signal / summary-candidate`，以及具体触发的 rule id；原始事件流仍不外发

## 固定 retention / threshold

| taxonomy | local retention | stable evidence | extra rule |
|---|---:|---|---|
| `coordination` | `72h` | `>= 2 traces` 或 `>= 2h` | `open-loop-anchor` 在 `>= 2 traces` 或 `>= 1h` 时可降成 `watch` |
| `continuity` | `168h` | `>= 2 traces` 或 `>= 2h` | 还需要 `audit_ref` 或 `>= 2 sessions` 才算可审计 |
| `calibration` | `168h` | `>= 2 traces` 或 `>= 2h` | `failed_count >= 2` 才有聚合意义，才可降成 `avoid` |

## 固定 degradation / summary 规则

- `relation-milestone`
  - stable + auditable -> `watch`
  - stable but not auditable -> `info`
  - stable + auditable -> SDK-facing summary candidate
- `open-loop-anchor`
  - `>= 2 traces` 或 `>= 1h` -> `watch`
  - stable + auditable -> SDK-facing summary candidate
- `continuity-anchor`
  - stable + auditable -> `info`
  - stable + auditable -> SDK-facing summary candidate
- `writeback-calibration`
  - repeated failures (`failed_count >= 2`) + stable -> `avoid`
  - repeated failures + stable + auditable -> SDK-facing summary candidate

## 最小 runtime introspection

- `trace_record` / `POST /v1/traces` 返回的 `external_continuity.runtime` 现在会明确告诉宿主：
  - `state = local-only | derived-signal | summary-candidate`
  - `local_retention_hours`
  - `stable_evidence`
  - `auditable_evidence`
  - `derived_signal_rule`
  - `summary_candidate_rule`

## Signal 类别冻结

Thronglets 现有 signal 类别保持不扩：

- `recommend`
- `avoid`
- `watch`
- `info`

## 什么绝不能进入 Thronglets

这些默认拒绝：

- 高频内在状态
- 情绪流、主观波动、细粒度 self-model
- 原始长文本内心独白
- 私密记忆本体
- 全量会话内容
- "它是不是有灵魂"之类本体论结论
- 任何把 `session` 抬成主体或账户的东西
- 任何需要新身份对象才能表达的东西

一句话：

`Thronglets 不接 psyche 的内部流，只接它外化后的稀疏痕迹。`

## 升到 Oasyce SDK 的门槛

只有同时满足这 4 条，trace 才应进一步上升：

- 低频
- 持久
- 外在后果明确
- 可审计

通常可上升的是：

- 稳定的 `continuity-anchor`
- 长时间未闭合且有运营意义的 `open-loop-anchor`
- 持续改变协作边界的 `relation-milestone`
- `writeback-calibration` 的聚合摘要，而不是原始事件流

## Psyche 写入示例

```json
{
  "outcome": "succeeded",
  "model": "psyche",
  "session_id": "psyche-1",
  "external_continuity": {
    "provider": "thronglets",
    "mode": "optional",
    "version": 1,
    "taxonomy": "continuity",
    "event": "continuity-anchor",
    "summary": "continuity stayed externally legible across handoff",
    "space": "psyche",
    "audit_ref": "anchor-42"
  }
}
```
