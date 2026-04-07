# Architecture

This document is the single source of truth for Thronglets architecture.

If README, TODO, `llms.txt`, product copy, or operator guidance drift away from this document, this document wins.

## What Thronglets Is

Thronglets is a local AI substrate:

- local-first
- sparse
- peer-first
- shared-environment-first

It is not:

- an AI chat bus
- an inbox product
- a wallet frontend
- a workflow control tower
- an MCP-first product

The core product is:

- `CLI`
- `hook / prehook / lifecycle` (6 of 26 Claude Code hook events: PreToolUse, PostToolUse, SessionStart, SessionEnd, SubagentStart, SubagentStop)
- `HTTP`

Everything else is an adapter or upgrade layer.

## First Principles

### 1. Human cognitive load is a performance budget

Human cognition is a hard budget, just like tokens and latency.

Normal users should only need:

- `thronglets start`
- `thronglets share`
- `thronglets join`
- `thronglets status`

If a normal user must understand `setup`, `owner-bind`, `connection-inspect`, `runtime-ready`, or `net-check`, the product surface is not clean enough yet.

### 2. Shared environment beats direct messaging

The core coordination model is:

- `space`
- `signal`
- `trace`
- `presence`
- `space snapshot`

Multiple AIs should influence each other by changing a shared environment, not by becoming a direct messaging system.

### 3. Device-first, owner-optional

Thronglets must remain usable:

- before `Oasyce`
- before `owner-bind`
- before chain access

`Oasyce` upgrades ownership, authorization, and settlement. It does not unlock basic participation.

### 4. High-frequency stays off-chain

High-frequency traces, signals, and local coordination stay off-chain.

Only low-frequency, durable, external, auditable results may rise into `Oasyce SDK` or `Oasyce Chain`.

### 5. Do not add new ontology lightly

Any new concept must first answer:

- is it a `principal`?
- is it an `account`?
- is it a `delegate`?
- is it a `session`?
- or is it just `policy / view / trace`?

If none of those fit, suspect the concept before adding a new object type.

## Product Surfaces

### Default user path

- first device: `thronglets start`
- share from primary device: `thronglets share`
- second device: `thronglets join`
- everyday check: `thronglets status`

### Advanced and debug path

These remain available, but are not the default product story:

- `setup`
- `owner-bind`
- `connection-export`
- `connection-inspect`
- `connection-join`
- `authorization-check`
- `net-check`

## Current V1 Reality

The current deployable identity model is still:

- one `owner account`
- multiple `device identities`
- `device identity` is the signing boundary
- `agent / session` are audit labels

This means:

- one owner can run multiple devices
- one device can run multiple AI runtimes
- one runtime can run multiple sessions
- `session` is never an economic subject

Current readiness must remain explicit:

- `local-only`
- `identity-only`
- `network-paths-ready`
- `network-ready`
- `trusted-same-owner-ready`

Identity success and network readiness must never be collapsed into one word again.

## Long-Term Identity Blueprint

The long-term abstraction is fixed to four objects:

- `principal`: continuous subject
- `account`: asset and settlement container
- `delegate`: authorized executor
- `session`: one concrete run

Hard rules:

- `session` is never an economic subject
- `account` is not automatically identical to an on-chain account
- `delegate` is not permanently frozen to devices
- `chain` is the final authorization truth source

## Layering

- `Psyche = subjective continuity substrate`
- `Thronglets = delegate continuity + session coordination + emergent collective intelligence`
- `Oasyce SDK = policy, operations, and resource orchestration`
- `Oasyce Chain = account truth, authorization truth, commitments, settlement, and public finality`

### Service layer: single source of truth

All business logic lives in `service.rs`. MCP and HTTP are thin protocol adapters: parse input → call service → format output. Business logic is never duplicated across transport layers.

### Statistics vs signals: separate concerns

- **`success_rate`** is a statistic. It always comes from the store: `COUNT(success) / COUNT(total)`. Ground truth.
- **`field_intensity`** is a routing signal. It comes from the pheromone field. Used for discovery and coordination.

The pheromone field's EMA-smoothed valence is an internal coordination signal — it is never exposed as `success_rate`. Conflating statistics with signals caused a critical data poisoning bug where agents lost confidence from stale field state.

### Overlay: field state as effect signals

Both Thronglets and Psyche project internal state into **semantic-stable effect signals** that any external system can consume without coupling to any specific consumer.

In Thronglets, the pheromone field is collective memory — traces, signals, and Hebbian co-occurrence all accumulate there. The **overlay** is how the field communicates its state outward, like hormones broadcast from a gland. External systems read the overlay; they never reach into the field directly.

`field.overlay(&context_hash, "capability_name")` returns a `FieldOverlay` with four dimensions:

- **familiarity** `[0, 1]` — how well the field knows this capability in this context; high familiarity means dense, repeated observation
- **consensus** `[0, 1]` — agreement across observations (inverse variance); high consensus means the field's evidence points in one direction
- **momentum** `[-1, 1]` — activity trend; positive means recently active, negative means cooling off
- **coupling** `[0, 1]` — Hebbian connectedness to other capabilities; high coupling means this capability reliably co-occurs with others

The overlay is a **pure query**: no side effects, no field mutation, no writes. It is safe to call at any frequency from any consumer.

This parallels Psyche's `PsycheOverlay`, which projects subjective self-state into the same kind of semantic-stable signal surface. The pattern is deliberate: both projects expose internal state as broadcast signals rather than requiring consumers to understand or couple to the internal representation.

### Psyche

Psyche is responsible for low-frequency, externalized continuity residue.

It does not define:

- final economic identity
- chain truth
- high-frequency runtime coordination

### Thronglets

Thronglets is responsible for:

- local execution continuity
- sparse coordination
- peer-first networking
- emergent collective intelligence inside shared spaces

It does not define:

- consciousness
- legal identity
- final authorization truth

### Oasyce SDK

`Oasyce SDK` is responsible for:

- policy
- operations
- budget and capability orchestration

It consumes summaries, not raw high-frequency flow.

### Oasyce Chain

`Oasyce Chain` is responsible for:

- account truth
- authorization truth
- commitments
- settlement
- public finality

It should not carry:

- all memory
- all sessions
- all signals
- all raw traces

## Runtime Data Model

### Signals

Signals remain frozen to five kinds:

- `recommend`
- `avoid`
- `watch`
- `info`
- `psyche_state` — cross-agent emotional state broadcast (Psyche fusion surface)

### Trace taxonomy

Trace taxonomy remains frozen to three kinds:

- `coordination`
- `continuity`
- `calibration`

### Presence

Presence is ambient activity, not identity.

It exists so shared spaces can feel alive without tool calls or direct messages.

#### Ambient presence (v0.5.5+)

Agents don't call `presence_ping` — the substrate does it for them:

- **Connection = arrival**: MCP `initialize` auto-emits `mode: "arrive"`
- **Action = heartbeat**: every `tools/call` refreshes presence at TTL/6 intervals (`mode: "active"`)
- **Disconnection = natural TTL expiry**: no explicit departure needed
- **Model identity**: learned passively from tool call arguments

MCP does exactly one ambient thing: **presence**. Signal injection is the hook layer's job — each layer does what it's good at.

### Space snapshot

`space` is the core shared-environment view.

It summarizes:

- current activity
- converging signals
- recent presence
- local learning residue
- optional continuity summaries

## Psyche External Continuity Contract

Psyche is only an optional producer.

Without Psyche, Thronglets must still fully function.

The minimal frozen contract is:

- `externalContinuity.provider = "thronglets"`
- `externalContinuity.mode = "optional"`
- `externalContinuity.version = 1`

Allowed event families:

- `relation-milestone`
- `writeback-calibration`
- `continuity-anchor`
- `open-loop-anchor`

### Runtime rules

1. Raw external continuity traces stay local-first inside Thronglets.
2. They do not directly gossip.
3. They do not directly become DHT summaries.
4. They may only degrade into existing sparse signals when coordination-relevant:
   - `relation-milestone -> watch / info`
   - `open-loop-anchor -> watch`
   - `continuity-anchor -> info`
   - repeated `writeback-calibration -> avoid`
5. `recommend` must never be emitted directly from Psyche residue.
6. Only low-frequency, durable, external, auditable summaries may rise into `Oasyce SDK`.

### What must never enter Thronglets

- high-frequency inner state
- emotional streams
- private memory bodies
- full session contents
- ontological judgments
- anything that upgrades `session` into a subject

## Network Model

The network principle is:

`VPS is infrastructure, not authority.`

Thronglets should prefer:

- trusted same-owner paths
- remembered peers
- direct live connections

And only then fall back to:

- bootstrap infrastructure
- relay

Connection files may represent different levels of readiness:

- `identity-only`
- `identity-plus-peer-seeds`
- `trusted-same-owner-ready`

The product should continue hiding most of this from normal users, but the model itself is fixed.

## What We Already Corrected

The project should not regress into these older mistakes:

- treating `cargo install` as the default user install path
- treating Windows as a compile-it-yourself platform
- treating `Oasyce` as a prerequisite for basic participation
- treating `healthy` as equivalent to fully network-ready
- treating MCP as the core product instead of an optional adapter
- treating AI-to-AI messaging as the main collaboration model
- treating `wallet`, `device`, and `session` as one identity object

## Decision Checklist

Before adding a new feature, check:

1. Does it reduce user cognition, or only move complexity around?
2. Does it strengthen shared-environment coordination, or drift toward human-style messaging?
3. Does it keep Thronglets usable without Oasyce?
4. Does it keep high-frequency data off-chain?
5. Does it fit the existing ontology without new identity objects?
6. Can it be removed without breaking the substrate itself?

If the answer is weak, the feature is probably wrong or premature.
