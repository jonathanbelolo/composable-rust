# Phase Final: Stream ID Architecture Fix

**Status**: 📋 Ready for Implementation
**Priority**: P0 - Critical Bug
**Impact**: Blocks all deployment tests (5/6 failing)

---

## Overview

This phase fixes a fundamental architectural bug where all instances of each aggregate type share a single event stream, causing optimistic concurrency conflicts.

## The Problem in One Image

```
❌ CURRENT (BROKEN):
All payments → stream "payment" (version conflicts)
All reservations → stream "reservation" (version conflicts)
All events → stream "event" (version conflicts)

✅ TARGET (CORRECT):
Payment #123 → stream "payment-123" (isolated)
Payment #456 → stream "payment-456" (isolated)
Reservation #789 → stream "reservation-789" (isolated)
```

## Documentation Structure

This directory contains complete planning documentation:

### 1. [STREAM_ID_FIX.md](./STREAM_ID_FIX.md) - Master Plan

**Contents**:
- Executive summary (what's wrong, why, impact)
- Current architecture analysis
- Target architecture design
- Phase-by-phase implementation strategy
- Open questions and investigation needs
- Success criteria and verification steps
- Risk assessment
- Implementation checklist

**Use**: Understand the problem, design decisions, and overall strategy

**Size**: ~2,800 lines

### 2. [INVESTIGATION_RESULTS.md](./INVESTIGATION_RESULTS.md) - Findings

**Contents**:
- Inventory stream ID pattern investigation (result: per-event)
- Catalog of all store creation call sites
- Saga coordinator analysis
- Test helper function identification

**Use**: Answers to open questions from the master plan

**Size**: ~300 lines

### 3. [IMPLEMENTATION_GUIDE.md](./IMPLEMENTATION_GUIDE.md) - Quick Reference

**Contents**:
- Before/after code examples for every pattern
- Store creation method updates
- CREATE endpoint pattern (new entity)
- READ endpoint pattern (existing entity)
- UPDATE/DELETE endpoint patterns
- Unit test patterns
- Compilation error reference
- Search commands for finding all call sites

**Use**: Copy-paste patterns during implementation, troubleshooting

**Size**: ~600 lines

### 4. [README.md](./README.md) - This File

**Contents**: Navigation and quick start

---

## Quick Start

### Phase 1: Read & Understand

1. **Read**: [STREAM_ID_FIX.md](./STREAM_ID_FIX.md) - Sections 1-3 (Executive Summary, Current Architecture, Target Architecture)
   - **Time**: 10 minutes
   - **Goal**: Understand what's broken and why

2. **Read**: [IMPLEMENTATION_GUIDE.md](./IMPLEMENTATION_GUIDE.md) - Pattern 1 & 2
   - **Time**: 5 minutes
   - **Goal**: See concrete before/after examples

### Phase 2: Prepare

1. **Verify Current State**:
   ```bash
   cd /Users/jonathanbelolo/dev/claude/code/composable-rust/examples/ticketing
   ./scripts/run-deployment-tests.sh
   ```
   - **Expected**: 5/6 tests pass, payment test fails
   - **Expected**: 45+ optimistic concurrency conflicts in logs

2. **Create Feature Branch**:
   ```bash
   git checkout -b fix/stream-id-per-instance
   ```

### Phase 3: Implement

Follow [STREAM_ID_FIX.md](./STREAM_ID_FIX.md) Section "Implementation Strategy":

**Step 1**: Update `src/server/state.rs` (4 store creation methods)
- Use [IMPLEMENTATION_GUIDE.md Pattern 1](./IMPLEMENTATION_GUIDE.md#pattern-1-store-creation-methods-serverstatrs)
- Compile: `cargo build --all-features` (expect many errors)

**Step 2**: Fix API endpoints (use compiler errors as guide)
- Use [IMPLEMENTATION_GUIDE.md Patterns 2-4](./IMPLEMENTATION_GUIDE.md#pattern-2-create-endpoints-new-entity)
- Fix one file at a time, compile after each

**Step 3**: Fix tests
- Use [IMPLEMENTATION_GUIDE.md Pattern 6](./IMPLEMENTATION_GUIDE.md#pattern-6-unit-tests)
- Compile: `cargo test --all-features`

**Step 4**: Verify
```bash
cargo build --all-features
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
```

### Phase 4: Validate

1. **Clean Database**:
   ```bash
   docker compose down -v
   docker compose up -d
   sleep 10  # Wait for services to be healthy
   ```

2. **Run Deployment Tests**:
   ```bash
   ./scripts/run-deployment-tests.sh
   ```

3. **Verify Success**:
   - ✅ 6/6 tests pass
   - ✅ 0 concurrency conflicts in logs
   - ✅ Event store has per-instance streams

4. **Inspect Event Store**:
   ```bash
   docker exec ticketing-events psql -U postgres -d ticketing_events -c \
     "SELECT DISTINCT stream_id FROM events ORDER BY stream_id LIMIT 20;"
   ```

   **Expected Output**:
   ```
   stream_id
   ----------------------------------------
   event-550e8400-e29b-41d4-a716-...
   event-660e8400-e29b-41d4-a716-...
   inventory-770e8400-e29b-41d4-a716-...
   payment-880e8400-e29b-41d4-a716-...
   reservation-990e8400-e29b-41d4-a716-...
   ```

---

## Files to Modify

### Core Infrastructure (1 file)

- ✏️ `src/server/state.rs` - Add entity ID parameters to 4 store creation methods

### API Endpoints (4 files)

- ✏️ `src/api/payments.rs` - 4 locations
- ✏️ `src/api/reservations.rs` - ~3 locations
- ✏️ `src/api/events.rs` - ~4 locations
- ✏️ `src/api/availability.rs` - 3 locations (inventory)

### Tests (TBD)

- ✏️ Unit tests in `src/aggregates/*/tests.rs`
- ✏️ Integration tests in `tests/*.rs`
- ✏️ Test helpers (if any)

**Estimate**: 12-15 file edits, ~30-40 line changes

---

## Key Decisions

### Inventory Stream ID Pattern

**Decision**: Per-event streams → `inventory-{event_id}`

**Rationale**:
- Each event has independent inventory
- Current `HashMap<(EventId, String), Inventory>` in state suggests multi-event support
- But in event sourcing, this violates aggregate-stream 1:1 relationship
- Correct pattern: One stream per event, state manages sections internally

**See**: [INVESTIGATION_RESULTS.md Section 1](./INVESTIGATION_RESULTS.md#question-1-inventory-stream-id-pattern)

### Stream ID Format

**Format**: `{aggregate_type}-{uuid}`

**Examples**:
- `payment-550e8400-e29b-41d4-a716-446655440000`
- `reservation-660e8400-e29b-41d4-a716-446655440001`
- `event-770e8400-e29b-41d4-a716-446655440002`
- `inventory-880e8400-e29b-41d4-a716-446655440003`

**Rationale**:
- Follows event sourcing best practices (aggregate = stream)
- Human-readable (can grep logs by aggregate type)
- UUID ensures uniqueness
- Consistent pattern across all aggregates

---

## Troubleshooting

### Compilation Errors

**Error**: `this function takes 2 arguments but 1 was supplied`

**Cause**: Forgot to pass entity ID to store creation method

**Fix**: See [IMPLEMENTATION_GUIDE.md Compilation Error Reference](./IMPLEMENTATION_GUIDE.md#compilation-error-reference)

### Tests Still Failing After Fix

**Check**:
1. Did you clear the database volumes? (`docker compose down -v`)
2. Are services healthy? (`docker compose ps`)
3. Any remaining singleton streams? (Check event store table)

**Debug**:
```bash
# Check for old singleton streams
docker exec ticketing-events psql -U postgres -d ticketing_events -c \
  "SELECT stream_id, version FROM events WHERE stream_id IN ('payment', 'reservation', 'event', 'inventory');"

# Should return 0 rows after fix
```

### Concurrency Conflicts Still Occur

**Check**:
1. Are tests running with `--test-threads=1`? (Prevents parallel test conflicts)
2. Is deployment script clearing database before tests?
3. Are you seeing conflicts on instance streams or singleton streams?

**Debug**:
```bash
# Check logs for conflict details
grep "Optimistic concurrency conflict" /tmp/ticketing-deployment-test.log | head -5

# Should see stream_id like "payment-{uuid}", not "payment"
# If still seeing "payment", fix not fully applied
```

---

## Success Metrics

**Before Fix**:
- ❌ 5/6 deployment tests pass
- ❌ 45+ optimistic concurrency conflicts
- ❌ Payment test times out after 10 seconds
- ❌ Event store has singleton streams: `payment`, `reservation`, `event`, `inventory`

**After Fix**:
- ✅ 6/6 deployment tests pass
- ✅ 0 optimistic concurrency conflicts
- ✅ Payment test completes successfully
- ✅ Event store has per-instance streams: `payment-{uuid}`, `reservation-{uuid}`, etc.

---

## Timeline

**Estimated Duration**: 4-6 hours

| Phase | Duration | Activity |
|-------|----------|----------|
| 1. Understanding | 30 min | Read documentation, understand problem |
| 2. Core Changes | 1 hour | Update server/state.rs, initial compilation |
| 3. API Endpoints | 2 hours | Fix all API endpoint call sites |
| 4. Tests | 1 hour | Fix unit tests and integration tests |
| 5. Verification | 30 min | Run tests, verify database, debug issues |
| 6. Validation | 30 min | Clean environment, deployment tests, final checks |

**Total**: ~5.5 hours (plus buffer)

---

## References

### Internal Documentation

- [STREAM_ID_FIX.md](./STREAM_ID_FIX.md) - Complete implementation plan
- [INVESTIGATION_RESULTS.md](./INVESTIGATION_RESULTS.md) - Investigation findings
- [IMPLEMENTATION_GUIDE.md](./IMPLEMENTATION_GUIDE.md) - Code examples and patterns

### Framework Documentation

- `../../docs/event-design-guidelines.md` - Event sourcing principles
- `../../docs/consistency-patterns.md` - Concurrency and consistency
- `../../docs/concepts.md` - Core architecture concepts

### External Resources

- **Event Sourcing**: https://martinfowler.com/eaaDev/EventSourcing.html
- **Aggregate Pattern**: https://martinfowler.com/bliki/DDD_Aggregate.html
- **Optimistic Concurrency**: https://en.wikipedia.org/wiki/Optimistic_concurrency_control

---

## Questions?

**Before Starting**: Re-read [STREAM_ID_FIX.md Executive Summary](./STREAM_ID_FIX.md#executive-summary)

**During Implementation**: Refer to [IMPLEMENTATION_GUIDE.md](./IMPLEMENTATION_GUIDE.md) for patterns

**When Stuck**: Check [Troubleshooting](#troubleshooting) section above

**Still Blocked**: Review [Investigation Results](./INVESTIGATION_RESULTS.md) for context

---

**Created**: 2025-11-23
**Status**: 📋 Ready for Implementation
**Next Step**: Begin with Phase 2 → Update `src/server/state.rs`
