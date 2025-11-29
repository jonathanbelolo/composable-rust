# composable-rust-next

Next-generation business logic framework for Composable Rust.

## Overview

This crate provides a clean separation between business logic and infrastructure,
designed as a compilation target for higher-level YAML-based specifications.

## Core Concepts

- **`BusinessLogic`**: Unified trait for aggregates and sagas
- **`BusinessResult`**: Return type indicating done or continue with calls
- **`Handler`**: Infrastructure orchestration (load, persist, broadcast)
- **`CallExecutor`**: Trait for saga call dispatch to aggregates

## Architecture

```text
┌─────────────────────────────────────────────────────────────┐
│  Handler (infrastructure, nearly identical across all)      │
│  ┌─────────────────────────────────────────────────────────┐│
│  │ 1. Load current state from event store                  ││
│  │ 2. Delegate to business logic                           ││
│  │ 3. Persist resulting events                             ││
│  │ 4. Broadcast for projections/sagas                      ││
│  └─────────────────────────────────────────────────────────┘│
│                           │                                 │
│                           ▼                                 │
│  ┌─────────────────────────────────────────────────────────┐│
│  │  BusinessLogic (domain-specific, pure)                   ││
│  │                                                         ││
│  │  process(state, input) → Result<BusinessResult, Error>  ││
│  │  apply(state, event) → mutate state                     ││
│  └─────────────────────────────────────────────────────────┘│
└─────────────────────────────────────────────────────────────┘
```

## Aggregates vs Sagas

Both use the same `BusinessLogic` trait. The difference:

- **Aggregates**: Always return `Done(events)`, use `Infallible` for `Call`/`CallResult`
- **Sagas**: Return `Continue { events, calls }` when orchestrating, `Done` when finished

## Status

🚧 **Under Development** - This is the next-generation framework that will eventually
replace `composable-rust-core` and `composable-rust-runtime`.

See `examples/ticketing/docs/compiler-target-architecture.md` for the full specification.
