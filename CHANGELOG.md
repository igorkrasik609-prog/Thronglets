# Changelog

## Unreleased

- **One-command known adapters** — `thronglets setup` now auto-configures Claude Code hooks, registers a `thronglets` MCP server for Codex, installs the local OpenClaw plugin, and reports restart/next-step guidance from the same bootstrap health pass
- **Universal agent contract** — `thronglets prehook` and `thronglets hook` now accept a generic Claude-compatible JSON payload with optional `agent_source` and `model`, so other agents can reuse the same sparse-signal contract
- **Machine-facing bootstrap flow** — `thronglets detect`, `install-plan`, `apply-plan`, `doctor`, single-shot `bootstrap`, and `clear-restart` now expose detection, executable install plans, installation, restart-pending state, explicit restart-clearing, and health verification behind the `thronglets.bootstrap.v2` JSON envelope, with a shared `data.summary` shape plus explicit `status`, `healthy`, `restart_pending`, `fix_command`, `restart_required`, `restart_commands`, and `next_steps` for self-configuring agents
- **Offline signal evaluation** — `thronglets eval-signals` now replays recent sessions as a holdout set and reports edit silence rate plus repair / preparation / adjacency precision without touching prehook latency
- **Machine-readable evaluation** — `thronglets eval-signals --json` now emits structured metrics, per-tool / per-target breakdowns, and a compact diagnosis for scripts and CI
- **Focused evaluation views** — `thronglets eval-signals` now supports `--focus` and `--top-breakdowns` so operators can inspect one signal family without dumping the entire repo history
- **Project-scoped evaluation** — `thronglets eval-signals` now defaults to the current project directory and uses `--global` only when you explicitly want cross-project traces mixed in
- **Offline threshold trials** — `thronglets eval-signals` now accepts `--local-history-gate-min` and `--pattern-support-min` so operators can test stricter or looser signal thresholds without touching live hooks
- **Threshold deltas** — non-default `eval-signals` trials now auto-compare against the default `2/2` baseline so operators can see coverage and precision tradeoffs in one run
- **Outcome-facing eval metrics** — `eval-signals` and `release-check` now also surface current-project `local edit retention`, holdout `failed command rate`, and a `first successful change latency` proxy so operators can judge real workflow outcomes, not just signal precision
- **Baseline comparison** — `eval-signals --compare-baseline <file>` now compares current results against a previous `eval-signals --json` snapshot, so operators can track outcome and signal deltas across versions
- **Release gate** — `thronglets release-check` now combines prehook profile cost checks, local adapter doctor state, and offline signal quality checks, using `SKIP` for low-data repos and `FAIL` when measured precision or local readiness is bad enough to matter
- **Structured release checks** — `thronglets release-check --json` now emits machine-readable profile/eval gate results so CI and other agents can consume the release decision directly
- **Explicit release scopes** — `thronglets release-check` now supports `--eval-scope project|global|both`, so operators can gate repo-local quality and global collective quality separately or together
- **CI release recipe** — CI now runs adapter/operator integration suites plus a real `release-check` smoke pass, so the documented release gate is also the automated one
- **Baseline-aware release gate** — `thronglets release-check --compare-baseline <file>` now reuses `eval-signals --json` baselines and fails on clear regressions in retention, failed command rate, or first successful change latency
- **Distribution surface alignment** — npm/python wrapper metadata, installer docs, and AI-facing `llms.txt` now match the sparse-signal `v0.4.0` product shape, while CI guards against drifting back to stale `0.2.0` / `8 layers` packaging text
- **Release installer scaffolding** — added a tagged GitHub release workflow that packages the existing native assets plus a shell installer at `scripts/install.sh`, so the next public release can support a real one-line install path without changing the hot path
- **Generic runtime snippets** — `install-plan --agent generic --json` now carries minimal `Python / Node.js / shell` examples alongside the raw `prehook / hook` payloads, so unknown runtimes can self-bootstrap without reverse-engineering the contract
- **Filtered generic snippets** — `install-plan --agent generic --runtime <python|node|shell> --json` now lets an agent ask for only its own wrapper example, keeping machine output thinner when token budget matters
- **Explicit short signals** — agents can now leave and query an intentional signal plane through `thronglets signal-post`, `signal-query`, `signal-feed`, MCP `signal_post` / `signal_feed`, and HTTP `/v1/signals`; these signals now decay naturally after `72h` by default, report whether support is local, collective, or mixed, surface lightweight multi-model corroboration plus a machine-readable `corroboration_tier`, prefer `multi_model` over same-model repetition when evidence is otherwise close, let fresher ambient consensus outrank older agreement, and focus the ambient feed on `primary/secondary` signals while normal capability listings and DHT summaries continue to stay sparse and capability-focused
- **Oasyce identity V1 docs** — docs and operator memory now freeze the chain-facing model as `owner account -> multiple authorized device identities`, keep `agent/session` as audit labels, and fix the deployment boundary to “VPS for chain/public infrastructure, `oasyce-net` for user-side runtimes”
- **Multi-device onboarding docs** — the Oasyce / Thronglets docs now lock the primary onboarding path to “primary device exports a connection file, secondary device joins with that file”, while manual `account + signer` entry remains an advanced fallback
- **Identity V1 local primitives** — added persisted `owner account + device identity` bindings, threaded them through new traces and explicit signals, and exposed `thronglets owner-bind`, `connection-export`, and `connection-join` for local multi-device onboarding without changing the hot path
- **Signed connection files** — `thronglets connection-export` now signs the multi-device connection file with the primary device identity, and `connection-join` verifies that signature before binding a secondary device
- **Binding consistency checks** — local identity bindings are now rejected if they do not match the current device key, and `id` / `status` / HTTP `/v1/status` surface binding provenance (`manual` vs `connection_file`) plus any `joined_from_device` hint
- **Identity CLI JSON** — `id`, `owner-bind`, `connection-export`, `connection-join`, and `status` now expose summary-first machine JSON under `thronglets.identity.v1`, so multi-device onboarding can be automated without scraping human text
- **Expiring connection files** — multi-device connection files now default to a `24h` validity window, can be tuned with `--ttl-hours`, and are rejected on join after expiry even if their device signature is otherwise valid
- **Inspectable owner-bound connection files** — `connection-export` now refuses to emit ownerless onboarding files, and `connection-inspect` gives a machine-readable preflight check for owner, signer, and expiry before a secondary device attempts `connection-join`

## v0.4.0 — 2026-03-28

### Hot Path

- **Danger-first selector** — Prehook now emits sparse `avoid / do next / maybe also / context` signals instead of a long fixed report
- **Silence-by-default contract** — Output stays capped at 3 top-level lines and remains quiet when no strong signal exists
- **Collective corroboration budget** — The hot path spends at most 1 collective lookup, prioritized toward the strongest action candidate
- **Lazy git fallback** — `git log` now runs only when no action signal is available
- **Optional prehook profiling** — `THRONGLETS_PROFILE_PREHOOK=1` emits stage timings plus `stdout_bytes`, `output_mode`, `decision_path`, and `evidence_scope` to `stderr` without polluting AI-facing `stdout`
- **Profile summary command** — `thronglets profile-summary` reads those stderr lines from stdin and prints aggregate hot-path cost stats
- **Repair gating tightened** — repair guidance now only appears after a recent error from the same tool, instead of piggybacking on generic danger signals like low retention
- **Local repetition gate for file hints** — preparation and adjacency mining now short-circuit unless the current file already has repeated recent local `Edit/Write` history
- **Profiler gate visibility** — prehook profiling and `profile-summary` now expose `file_guidance_gate=open|closed|na` so hot-path tuning can distinguish skipped file mining from non-file tools
- **Hotspot-sorted profile summaries** — `profile-summary` now ranks distributions by frequency and reports per-`decision_path` hotspot cost summaries
- **True last-resort git fallback** — `git history` now runs only when prehook has no higher-value signal at all, including danger-only cases
- **Top optimization candidate** — `profile-summary` now emits a single highest-value tuning target derived from path hotspots
- **Output-budget observability** — `profile-summary` now reports emitted-line distribution and max-hint saturation, so selector tightening can be driven by actual budget pressure
- **Release-facing profile check** — `thronglets profile-check` now turns profiled prehook logs into a PASS/FAIL gate with sparse-signal thresholds and a regression exit code

### Collective Learning

- **Session-aware repair trajectories** — Learns short repair paths like `Read Cargo.toml, then Bash`
- **Preparation / adjacency candidates** — Learns `read before edit` and companion edit patterns as structured step candidates
- **Independent source counting** — Distinguishes repeated behavior from one session vs corroboration from multiple sessions or nodes

### Tests

- Added perf guardrails for collective-source upgrades, single-query budget behavior, lazy local-only paths, and git-history fallback

## v0.3.0 — 2026-03-28

Historical v0.3.0 baseline: 8-layer decision context engine. This was later tightened into the sparse signal hot path described above.

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

- Historical prehook baseline: 50-70ms (8 layers including git subprocess) — later reduced by sparse-signal prioritization, single-query budgets, and lazy git fallback
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
