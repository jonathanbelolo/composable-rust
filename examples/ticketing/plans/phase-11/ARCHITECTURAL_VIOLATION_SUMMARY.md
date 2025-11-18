# Architectural Violation Summary - Ticketing Example

**Date:** 2025-11-17
**Severity:** 🔴 CRITICAL - Fundamental architecture violation
**Status:** Requires immediate remediation

---

## The Problem

The ticketing example **violates the core Composable Rust architecture** by introducing a service layer that:

1. **Ignores reducer-returned effects** - Effects are discarded with `let _effects = ...`
2. **Makes its own persistence decisions** - Services decide to save/publish instead of executing what the reducer requests
3. **Persists commands instead of events** - Saves the input action rather than events emitted by the reducer
4. **Breaks the feedback loop** - Effects like `Effect::Delay` and `Effect::Future` are never executed

## What Was Built (WRONG)

```rust
// src/app/services.rs
impl InventoryService {
    pub async fn handle(&self, action: InventoryAction) -> Result<()> {
        // 1. Load state
        // 2. Call reducer
        let _effects = self.reducer.reduce(&mut state, action.clone(), &self.env);
        //     ^^^^^^^^ EFFECTS IGNORED! ❌

        // 3. Service makes its own decision to persist
        self.event_store.append_events(...).await?;

        // 4. Service makes its own decision to publish
        self.event_bus.publish(&self.topic, &serialized).await?;

        Ok(())
    }
}
```

**Problems:**
- ❌ Effects returned by reducer are thrown away
- ❌ Service hardcodes "save then publish" behavior
- ❌ No support for `Effect::Delay` (timeouts don't work)
- ❌ No support for `Effect::Future` (saga coordination broken)
- ❌ Reducer has no control over side effects
- ❌ Business logic leaks into infrastructure layer

## What Should Exist (CORRECT)

Per the **Composable Rust architecture** documented in `.claude/skills/composable-rust-architecture/SKILL.md`:

```rust
// The Store IS the runtime - no service layer needed
use composable_rust_runtime::Store;

// HTTP Handler talks directly to Store
pub async fn create_reservation(
    State(store): State<Arc<Store<...>>>,
    Json(req): Json<CreateReservationRequest>,
) -> Result<Json<Response>> {
    let command = ReservationAction::InitiateReservation { ... };

    // Store handles everything:
    // 1. Calls reducer
    // 2. Executes returned effects
    // 3. Feeds resulting actions back to reducer
    store.send(command).await;

    Ok(Json(response))
}
```

**The Store already:**
- ✅ Executes `Effect::None`, `Effect::Future`, `Effect::Delay`, `Effect::Parallel`, `Effect::Sequential`
- ✅ Manages the action feedback loop
- ✅ Provides observability (metrics, tracing, DLQ)
- ✅ Handles retry policies and circuit breakers

## Why This Matters

This violation breaks **all five core principles** of Composable Rust:

1. **Functional Core, Imperative Shell** - Business logic (reducer) now can't describe side effects
2. **Unidirectional Data Flow** - The feedback loop is broken
3. **Explicit Effects** - Effects are invisible (ignored by services)
4. **Dependency Injection** - Services hardcode behavior instead of executing reducer's requests
5. **Composability** - Can't compose effects (Parallel, Sequential) because they're ignored

## Impact on Features

| Feature | Status | Reason |
|---------|--------|--------|
| Event persistence | ⚠️ Works accidentally | Service hardcodes it, not via Effect |
| Reservation timeouts | ❌ Broken | `Effect::Delay` ignored |
| Saga coordination | ❌ Broken | `Effect::Future` with cross-aggregate commands ignored |
| Payment processing | ⚠️ Partial | Always succeeds, no effect-based gateway integration |
| Event sourcing | ⚠️ Accidental | Commands saved, not events |

## How This Happened

**Available documentation:**
- ✅ 7 expert skills in `.claude/skills/` (5,250+ lines)
- ✅ CLAUDE.md explicitly states "Store executes effects"
- ✅ `runtime/src/lib.rs` documents effect execution (lines 1-40)
- ✅ Architecture skill shows correct patterns

**What went wrong:**
- Previous Claude instance built a traditional "service layer" (Spring/NestJS pattern)
- Didn't consult or follow the Composable Rust skills
- Implemented what "felt familiar" instead of what was documented
- Integration tests passed (service layer does persist) giving false confidence

## Immediate Actions Required

See **REMEDIATION_PLAN.md** for detailed steps to:

1. Remove the service layer (`src/app/services.rs`)
2. Make reducers return proper effects
3. Use Store directly from HTTP handlers
4. Restore the Composable Rust architecture

---

**Status:** Remediation plan ready
**Next Steps:** Execute remediation plan in Phase 11
