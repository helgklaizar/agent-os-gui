# agentd A-0003. Model providers: a thin layer with two adapters

Status: accepted

## Context

The product spec: cloud models (Anthropic, any OpenAI-compatible endpoint, 75+ models via the models.dev catalog); local models (Ollama, llama.cpp) with hardware autodetect; model roles (chat, observer, code, feed), keys and fallback. [ADR-0008](0008-open-questions-closed.md) closed the quota question as **accounting by fact**: tokens and cost come from the `usage` field of responses, API key limits (requests and tokens per minute) from rate-limit headers, and GPU/VRAM load for local models. Subscription balance is not shown; rotation between subscriptions is dropped; rotation between API keys of one owner is after 0.1.

Research recommendations: do not write a provider framework, take a thin layer with two adapters (Anthropic and OpenAI-compatible); LiteLLM as a dependency is risky (its PyPI package was compromised in March 2026); the Claude Agent SDK does not fit as a core. The DB structure for accounting is in [db](../reference/db.md) (`quotas`, `quota_samples`). Keys live in a `0600` env file written through `PUT /secrets/{ref}`, referenced by `accounts.secret_ref` ([ADR-0002](0002-db-single-writer.md)).

## Decision

1. **The `providers` crate** holds a `Provider` trait and two adapters: `anthropic` (Messages API) and `openai_compat` (Chat Completions by base URL and key). Ollama and llama.cpp connect as `openai_compat` on a local address; their OpenAI-compatible endpoints are taken from those projects' public documentation and are not verified here. The model catalog (models.dev) is data for model selection in Settings, not an adapter; how it is fetched and cached is undefined.
2. **Interface (proposal):** `stream(request) -> event stream`, where an event is a text delta, a tool request (`tool_use`), usage metadata, end, or error. The request carries model, history, tool definitions and limits. The normalized representation is shared by both adapters; provider details do not leave the crate (the GUI does not know about providers, [ADR-0001](0001-init-stack.md)).
3. **Usage accounting:**
   - `usage` from each response is written to `quotas.usage_tokens`, `usage_cost` and `quota_samples` (formulas in [db](../reference/db.md));
   - rate-limit headers give `limit_requests`, `limit_tokens`, `remaining`, `reset_at`; header names come from provider documentation and are not verified. Until a response with headers has been seen, the fields stay `NULL` and the GUI shows usage only;
   - for local models `kind = local` and `load` comes from GPU and VRAM metrics (the `monitor` component, sysfs and NVML);
   - the "runs out in" forecast (`eta_at`) is computed only when remaining and reset are known and only if the ETA is earlier than the reset ([api](../reference/api.md)).
   Cost is computed from a price table; its source is not defined.
4. **Keys.** The adapter receives the key value from `agentd` for the duration of a request, never logs it and never keeps it longer. Provider errors are normalized: `auth` (key rejected), `rate_limit` (with `retry_after`), `overloaded`, `invalid_request`, `transport`; the GUI gets the code and class, not the provider response body.
5. **Fallback** (local model / another key / wait): `agentd` decides at the orchestrator level from the error class and the persona setting. Round-robin rotation between accounts is not implemented. Order and thresholds are not defined.
6. **Subscriptions.** Subscription login (claude.ai OAuth) is not used: there is no official balance API, and the Agent SDK forbids claude.ai login in third-party products. API keys only.
7. **The observer** (`agent-watch`) talks to a local model; cloud keys are never passed to it.
8. **Testing.** Adapters are tested against recorded SSE fixtures and a local stub HTTP server in `tests/`; real providers are unavailable in automated tests. An end-to-end test with a real provider is manual.

## Consequences

- Two adapters cover cloud and local models; a provider with a non-standard API means a new adapter and, per the dependency policy, another ADR.
- No dependency on aggregators (LiteLLM) or the Agent SDK.
- Cost accuracy is limited by what providers return; subscription balance is unavailable by design.
- OpenAI compatibility across providers is incomplete (differing `tool_calls`, `usage` in streams); each connected provider needs verification, effort not estimated.
- The design of rate-limit headers and streamed `usage` must be checked against provider documentation before implementation (not done).

## Alternatives

- **LiteLLM (proxy or library).** Rejected: Python dependency and supply-chain risk.
- **A ready-made Rust provider aggregator crate.** Not examined in detail; the dependency policy requires its criteria and an ADR for a new layer.
- **Claude Agent SDK as provider and loop.** Rejected: launches the Claude Code binary, vendor lock-in, claude.ai login restrictions.
- **OpenAI-compatible adapter only.** Possible as a start, but loses Anthropic specifics (event stream, `usage`, headers); not chosen.
- **Scraping subscription balance.** Rejected.
