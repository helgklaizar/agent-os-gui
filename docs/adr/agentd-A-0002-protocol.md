# agentd A-0002. Client to agentd protocol: REST + WebSocket stays

Status: accepted

## Context

The [api](../reference/api.md) contract and its OpenAPI draft describe REST (`/v1`, about 50 paths) and a WebSocket `/v1/ws` with topics, snapshots, `seq` and deltas. The mock core, contract tests and the client library are built on it.

A research note suggested replacing the custom 50+ paths with JSON-RPC 2.0 over socket and WebSocket on ACP schemas, adding project-specific features (bar, quotas, personas, modules) as `_agentos/*` extensions. Relevant facts:

- ACP v1 defines only stdio; remote transport is a draft, and v2 is a draft.
- Goose is moving from its own REST+SSE (103 endpoints) to ACP with its own extension methods.
- Adopting ACP wholesale is early and would break the current API document, the OpenAPI draft and the client library.
- The stack stays as in [ADR-0001](0001-init-stack.md); any stack change needs a separate ADR.

## Decision

1. **For 0.1 the protocol stays as in [api](../reference/api.md): REST `/v1` plus WebSocket `/v1/ws`**, with tokens and scopes per [ADR-0003](0003-api-auth.md). `agentd` implements the contract; a mismatch with the implementation is fixed in the document, not silently in code.
2. **ACP comes after 0.1** and only as an additional layer. Conditions for revisiting: ACP has a fixed remote transport, there are clients (editors, CLIs) that justify it, and the migration cost has been measured.
3. **The service layer does not know the transport.** `api` handlers stay thin: parse the request and call services in `core` (typed commands and queries). A second facade (`acp`, JSON-RPC over socket and WebSocket) can then be added as a separate crate over the same services without touching the orchestrator, permissions or DB ([architecture](../architecture.md)).
4. **The same contract tests** run against the mock core and against `agentd` (a proposal), so the contract remains the single source of truth for both.
5. **Extensions are named in advance:** if ACP is added, custom methods live under `_agentos/*`; compatible additions do not change the `/v1` version.

## Consequences

- The mock core, the OpenAPI draft and the client library are not revisited.
- `agentd` implements its own set of paths and topics (more work than an off-the-shelf ACP adapter); the contract is maintained in both `agentd` and the mock core.
- Third-party ACP clients cannot talk to the core in 0.1.
- Rule 3 gives an extension point but needs discipline: business logic in `api` handlers is forbidden (checked in review).
- Risk (not measured): a custom protocol of 50+ paths is costly to maintain; Goose reached that conclusion at 103 endpoints.

## Alternatives

- **JSON-RPC 2.0 on ACP schemas with `_agentos/*` extensions now.** Rejected for 0.1: breaks the current contract, ACP v1 lacks remote transport. Remains a candidate after 0.1.
- **gRPC only.** Not examined; likely incompatible with the token scheme and WebKit clients without an extra layer (not verified).
- **SSE instead of WebSocket.** Not examined; WebSocket was chosen for two-way subscriptions and `ping`/`pong`.
- **REST without WebSocket, polling.** Rejected: the contract uses snapshot plus deltas, `seq` and resync.
