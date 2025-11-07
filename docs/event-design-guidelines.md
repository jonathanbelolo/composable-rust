# Event Design Guidelines

**Best Practices for Designing Events in Event-Sourced Systems**

> 📖 **Companion Document**: This guide focuses on event schema design. For consistency patterns, see [Consistency Patterns](./consistency-patterns.md).

---

## Table of Contents

1. [Overview](#overview)
2. [Fat Events vs Thin Events](#fat-events-vs-thin-events)
3. [Event Naming Conventions](#event-naming-conventions)
4. [Data Inclusion Guidelines](#data-inclusion-guidelines)
5. [Event Versioning](#event-versioning)
6. [Schema Evolution Patterns](#schema-evolution-patterns)
7. [Serialization Considerations](#serialization-considerations)
8. [Performance Trade-offs](#performance-trade-offs)
9. [Testing Event Schemas](#testing-event-schemas)
10. [Best Practices](#best-practices)

---

## Overview

Events are the **source of truth** in event-sourced systems. Well-designed events make systems:

- **Easier to understand**: Clear intent and data
- **Easier to extend**: Schema evolution support
- **Easier to debug**: Complete audit trail
- **Faster to process**: No additional queries needed

### Key Principles

> **1. Events are facts** - Past tense, immutable
>
> **2. Events are complete** - Include all data consumers need
>
> **3. Events are versioned** - Support schema evolution
>
> **4. Events are serializable** - Bincode or JSON

---

## Fat Events vs Thin Events

The most important design decision: how much data to include in events?

### Thin Events (Anti-Pattern)

Events with minimal data (IDs only):

```rust
// ❌ BAD: Thin event
#[derive(Clone, Serialize, Deserialize)]
pub struct OrderPlacedEvent {
    pub order_id: OrderId,
    pub customer_id: CustomerId,
    pub timestamp: DateTime<Utc>,
    // ❌ Missing: items, total, addresses, payment method
}
```

**Problems**:
1. **Forces queries**: Consumers must query to get order details
2. **Race conditions**: Projection may not be updated yet
3. **Tight coupling**: Services depend on each other's data stores
4. **Higher latency**: Extra round trips
5. **Harder to replay**: Can't reconstruct state without queries

**Example Problem**:

```rust
// Consumer forced to query:
async fn handle_order_placed(&mut self, event: OrderPlacedEvent) -> Result<()> {
    // ❌ Must query to get order details
    let order = self.projection.get_order(&event.order_id).await?;

    // Race condition: projection might not be updated yet!
    if let Some(order) = order {
        self.process_order(order).await?;
    } else {
        // Order was just created but projection not updated yet
        return Err("Order not found".into());  // False negative!
    }
}
```

### Fat Events (Correct Pattern)

Events with complete data:

```rust
// ✅ GOOD: Fat event
#[derive(Clone, Serialize, Deserialize)]
pub struct OrderPlacedEvent {
    // Identifiers
    pub order_id: OrderId,
    pub customer_id: CustomerId,

    // Order details (complete)
    pub items: Vec<LineItem>,              // ✅ Full item details
    pub subtotal: Money,                   // ✅ Pre-calculated
    pub tax: Money,                        // ✅ Pre-calculated
    pub shipping_cost: Money,              // ✅ Pre-calculated
    pub total: Money,                      // ✅ Grand total

    // Addresses (complete)
    pub shipping_address: Address,         // ✅ Full address
    pub billing_address: Address,          // ✅ Full address

    // Payment (complete)
    pub payment_method: PaymentMethod,     // ✅ Full payment details

    // Optional data
    pub discount_code: Option<String>,     // ✅ Applied discount
    pub customer_note: Option<String>,     // ✅ Special instructions

    // Metadata
    pub timestamp: DateTime<Utc>,
    pub version: u32,
}

// Supporting types (also complete)
#[derive(Clone, Serialize, Deserialize)]
pub struct LineItem {
    pub product_id: ProductId,
    pub product_name: String,              // ✅ Denormalized
    pub product_sku: String,               // ✅ Denormalized
    pub quantity: u32,
    pub unit_price: Money,
    pub total_price: Money,                // ✅ Pre-calculated
    pub tax_rate: Decimal,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Address {
    pub name: String,
    pub street1: String,
    pub street2: Option<String>,
    pub city: String,
    pub state: String,
    pub postal_code: String,
    pub country: String,
    pub phone: Option<String>,
}
```

**Benefits**:
1. **No queries needed**: Event is self-contained
2. **No race conditions**: All data in event
3. **Loose coupling**: Services are independent
4. **Lower latency**: No extra round trips
5. **Easier to replay**: Complete history in events

**Example Usage**:

```rust
// Consumer can process immediately:
async fn handle_order_placed(&mut self, event: OrderPlacedEvent) -> Result<()> {
    // ✅ All data is in the event!
    self.state.order_id = event.order_id;
    self.state.customer_id = event.customer_id;
    self.state.items = event.items;
    self.state.order_total = event.total;
    self.state.shipping_address = event.shipping_address;
    self.state.payment_method = event.payment_method;

    // Process immediately - no queries, no race conditions
    self.charge_payment(event.payment_method, event.total).await?;
    Ok(())
}
```

### When to Use Each

| Use Case | Thin Events | Fat Events |
|----------|-------------|------------|
| **Workflow coordination** | ❌ No | ✅ Yes |
| **Saga decision-making** | ❌ No | ✅ Yes |
| **Projection updates** | Maybe | ✅ Preferred |
| **Audit trail** | ❌ Incomplete | ✅ Complete |
| **Event replay** | ❌ Requires queries | ✅ Self-contained |
| **Storage cost** | Lower | Higher |

**Verdict**: **Always use fat events for critical workflows.** Storage is cheap, race conditions are expensive.

---

## Event Naming Conventions

### Rule 1: Past Tense

Events describe what **happened**, not what **should** happen:

```rust
// ✅ GOOD: Past tense
OrderPlacedEvent
PaymentChargedEvent
InventoryReservedEvent
ShipmentScheduledEvent

// ❌ BAD: Present/future tense
PlaceOrderEvent      // Command, not event
ChargePaymentEvent   // Command, not event
ReserveInventoryEvent // Command, not event
```

### Rule 2: Specific and Descriptive

Be precise about what happened:

```rust
// ✅ GOOD: Specific
CustomerRegisteredEvent
OrderCancelledEvent
PaymentRefundedEvent

// ❌ BAD: Generic
CustomerEventEvent  // Redundant "Event"
OrderUpdatedEvent   // Too vague - updated how?
PaymentEvent        // Too vague - what happened?
```

### Rule 3: Domain Language

Use ubiquitous language from the domain:

```rust
// E-commerce domain
// ✅ GOOD: Domain terms
OrderPlacedEvent
OrderShippedEvent
OrderDeliveredEvent

// Banking domain
// ✅ GOOD: Domain terms
AccountOpenedEvent
FundsDepositedEvent
FundsWithdrawnEvent
TransferInitiatedEvent

// ❌ BAD: Technical jargon
CreateOrderEvent    // "Create" is technical
InsertOrderEvent    // "Insert" is database term
```

### Rule 4: Versioned Names (When Needed)

When events evolve significantly:

```rust
// Version 1
OrderPlacedEvent

// Version 2 (breaking change)
OrderPlacedV2Event

// Better: Use version field instead
#[derive(Clone, Serialize, Deserialize)]
pub struct OrderPlacedEvent {
    #[serde(default = "default_version")]
    pub version: u32,  // 1, 2, 3, ...

    // Event data...
}

fn default_version() -> u32 { 1 }
```

### Section 3: Automatic Event Type Generation

**Section 3 adds `#[derive(Action)]` to auto-generate versioned event types:**

```rust
use composable_rust_macros::Action;

#[derive(Action, Clone, Serialize, Deserialize, Debug)]
enum OrderAction {
    // Commands
    #[command]
    PlaceOrder { customer_id: String, items: Vec<LineItem> },

    // Events (marked with #[event])
    #[event]
    OrderPlaced { order_id: String, timestamp: DateTime<Utc> },

    #[event]
    OrderShipped { order_id: String, tracking: String },

    #[event]
    OrderCancelled { order_id: String, reason: String },
}

// Auto-generated method:
let event = OrderAction::OrderPlaced { /* ... */ };
assert_eq!(event.event_type(), "OrderPlaced.v1");  // ✅ Versioned by default!
```

**Benefits**:
- **Automatic versioning**: `.v1` suffix added automatically for schema evolution
- **CQRS enforcement**: Compile-time distinction between commands and events
- **Zero boilerplate**: No manual event type string constants

**See also**: [API Reference](api-reference.md#derive-macro-action) for full documentation.

---

## Data Inclusion Guidelines

### What to Include in Events

#### ✅ Always Include

1. **Identifiers**:
```rust
pub order_id: OrderId,
pub customer_id: CustomerId,
pub product_id: ProductId,
```

2. **Core data** (what changed):
```rust
pub items: Vec<LineItem>,
pub total: Money,
pub status: OrderStatus,
```

3. **Metadata**:
```rust
pub timestamp: DateTime<Utc>,
pub version: u32,
pub correlation_id: Option<Uuid>,  // For tracing
```

4. **Denormalized lookups** (for consumers):
```rust
pub customer_name: String,        // Not just customer_id
pub product_name: String,          // Not just product_id
pub product_sku: String,
```

5. **Pre-calculated values**:
```rust
pub subtotal: Money,
pub tax: Money,
pub shipping_cost: Money,
pub total: Money,                  // Pre-calculated grand total
```

6. **Complete nested objects** (addresses, line items):
```rust
pub shipping_address: Address,     // Complete, not just address_id
pub items: Vec<LineItem>,          // Complete details, not just IDs
```

#### ❓ Consider Including

7. **Causation data** (why did this happen?):
```rust
pub reason: Option<String>,        // Order cancelled - why?
pub triggered_by: Option<UserId>,  // Who/what triggered this?
```

8. **Previous state** (for debugging):
```rust
pub previous_status: Option<OrderStatus>,
pub previous_total: Option<Money>,
```

#### ❌ Don't Include

9. **Sensitive data** (unless encrypted):
```rust
// ❌ BAD: Unencrypted sensitive data
pub credit_card_number: String,    // Don't store!
pub ssn: String,                   // Don't store!

// ✅ GOOD: Tokenized or last 4 digits
pub payment_token: String,         // Tokenized
pub card_last_four: String,        // Last 4 digits only
```

10. **Large binary data** (store separately):
```rust
// ❌ BAD: Large files in events
pub product_image: Vec<u8>,        // Can be MBs!

// ✅ GOOD: Reference to storage
pub product_image_url: String,     // S3/CDN URL
```

11. **Computed aggregations** (recalculate instead):
```rust
// ❌ BAD: Aggregate in event
pub total_orders_today: u64,       // Out of date immediately

// ✅ GOOD: Query projection instead
// Calculate from projection on demand
```

### Checklist: Event Data Inclusion

For each event, ask:

- [ ] Do sagas need this data?
- [ ] Do projections need this data?
- [ ] Can consumers process without querying?
- [ ] Is this data available when event is created?
- [ ] Will this data be useful for debugging?
- [ ] Is the event self-contained?

If yes to most, **include it**.

---

## Event Versioning

Events are immutable, but schemas evolve. Support versioning from day one.

### Pattern 1: Version Field

Add version field to all events:

```rust
#[derive(Clone, Serialize, Deserialize)]
pub struct OrderPlacedEvent {
    // ✅ Version field (default to 1)
    #[serde(default = "default_version")]
    pub version: u32,

    // Event data...
    pub order_id: OrderId,
    pub customer_id: CustomerId,
    pub items: Vec<LineItem>,
    pub total: Money,
    pub timestamp: DateTime<Utc>,
}

fn default_version() -> u32 { 1 }
```

**Benefits**:
- Easy to identify event version
- Can handle multiple versions in same system
- Forward-compatible (new code reads old events)

### Pattern 2: Optional Fields (Additive Changes)

Adding fields is safe if they're optional:

```rust
// Version 1
#[derive(Clone, Serialize, Deserialize)]
pub struct OrderPlacedEvent {
    pub version: u32,  // 1
    pub order_id: OrderId,
    pub customer_id: CustomerId,
    pub items: Vec<LineItem>,
    pub total: Money,
    pub timestamp: DateTime<Utc>,
}

// Version 2: Added optional field
#[derive(Clone, Serialize, Deserialize)]
pub struct OrderPlacedEvent {
    #[serde(default = "default_version_2")]
    pub version: u32,  // 2

    pub order_id: OrderId,
    pub customer_id: CustomerId,
    pub items: Vec<LineItem>,
    pub total: Money,
    pub timestamp: DateTime<Utc>,

    // ✅ NEW: Optional field (defaults to None for v1 events)
    #[serde(default)]
    pub discount_code: Option<String>,
}

fn default_version_2() -> u32 { 2 }
```

**Reading old events**:
```rust
// Old event (v1):
// { "version": 1, "order_id": "...", "total": 100 }
//
// Deserializes to:
// OrderPlacedEvent {
//     version: 1,
//     order_id: "...",
//     total: 100,
//     discount_code: None,  // ✅ Defaults to None
// }
```

### Pattern 3: Upcasting (Breaking Changes)

When events change significantly, upcast old events:

```rust
// Version 1 schema
#[derive(Clone, Serialize, Deserialize)]
pub struct OrderPlacedEventV1 {
    pub order_id: OrderId,
    pub items: Vec<String>,  // Just product IDs
    pub total: Money,
}

// Version 2 schema (breaking change)
#[derive(Clone, Serialize, Deserialize)]
pub struct OrderPlacedEventV2 {
    pub order_id: OrderId,
    pub items: Vec<LineItem>,  // ✅ Full line items (breaking change)
    pub total: Money,
    pub shipping_address: Address,  // ✅ New required field
}

// Upcast v1 → v2
impl From<OrderPlacedEventV1> for OrderPlacedEventV2 {
    fn from(v1: OrderPlacedEventV1) -> Self {
        Self {
            order_id: v1.order_id,
            items: v1.items.into_iter().map(|product_id| {
                // ✅ Upcast: Create LineItem from product_id
                LineItem {
                    product_id: ProductId::new(product_id),
                    product_name: "Unknown".to_string(),  // Best effort
                    quantity: 1,
                    unit_price: Money::zero(),
                    total_price: Money::zero(),
                }
            }).collect(),
            total: v1.total,
            shipping_address: Address::default(),  // ✅ Default for missing field
        }
    }
}

// Deserialize and upcast:
pub fn deserialize_order_placed(data: &[u8]) -> Result<OrderPlacedEventV2> {
    // Try v2 first
    if let Ok(event) = bincode::deserialize::<OrderPlacedEventV2>(data) {
        return Ok(event);
    }

    // Fall back to v1 and upcast
    let v1 = bincode::deserialize::<OrderPlacedEventV1>(data)?;
    Ok(v1.into())
}
```

---

## Schema Evolution Patterns

### ✅ Safe Changes (Non-Breaking)

1. **Adding optional fields**:
```rust
// ✅ SAFE: New optional field
#[serde(default)]
pub discount_code: Option<String>,
```

2. **Adding new event types**:
```rust
// ✅ SAFE: New event type
pub enum OrderAction {
    OrderPlaced { /* ... */ },
    OrderShipped { /* ... */ },
    OrderRefunded { /* ... */ },  // ✅ New event type
}
```

3. **Deprecating (but not removing) fields**:
```rust
#[deprecated(since = "1.2.0", note = "Use `items` instead")]
pub product_ids: Option<Vec<String>>,

pub items: Vec<LineItem>,  // Replacement field
```

### ⚠️ Breaking Changes (Require Upcasting)

1. **Removing fields**:
```rust
// ❌ BREAKING: Removed field
// pub old_field: String,  // Can't read old events without this!
```

2. **Changing field types**:
```rust
// ❌ BREAKING: Changed type
// Before: pub total: f64
pub total: Money,  // After: Different type
```

3. **Renaming fields**:
```rust
// ❌ BREAKING: Renamed field
// Before: pub customer_id: String
pub purchaser_id: String,  // After: Different name
```

**Solution**: Use upcasting (see Pattern 3 above)

### Event Evolution Strategy

```
Year 1: v1 events (100% of events)
        ↓
Year 2: v2 events introduced
        - New events use v2
        - Old events remain v1
        - System reads both (upcasting)
        ↓
Year 3: Optional: Migrate v1 → v2
        - Background job rewrites old events
        - Or keep both forever (storage is cheap)
```

---

## Serialization Considerations

### Bincode vs JSON

| Aspect | Bincode | JSON |
|--------|---------|------|
| **Size** | Smaller (30-70%) | Larger |
| **Speed** | Faster (5-10x) | Slower |
| **Human-readable** | No | Yes |
| **Schema evolution** | Harder | Easier |
| **Debugging** | Harder | Easier |
| **Recommendation** | Production | Development |

### Bincode Configuration

```rust
use bincode::config;

// Production configuration
let config = config::standard()
    .with_little_endian()
    .with_variable_int_encoding();

// Serialize
let bytes = bincode::encode_to_vec(&event, config)?;

// Deserialize
let event: OrderPlacedEvent = bincode::decode_from_slice(&bytes, config)?.0;
```

### JSON Configuration

```rust
use serde_json;

// Serialize (pretty for debugging)
let json = serde_json::to_string_pretty(&event)?;

// Deserialize
let event: OrderPlacedEvent = serde_json::from_str(&json)?;
```

### Serde Attributes for Schema Evolution

```rust
#[derive(Clone, Serialize, Deserialize)]
pub struct OrderPlacedEvent {
    // Default value for missing field
    #[serde(default)]
    pub discount_code: Option<String>,

    // Rename field (backward compatible)
    #[serde(rename = "customerId", alias = "customer_id")]
    pub customer_id: CustomerId,

    // Skip serializing None
    #[serde(skip_serializing_if = "Option::is_none")]
    pub customer_note: Option<String>,

    // Custom serialization
    #[serde(serialize_with = "serialize_money", deserialize_with = "deserialize_money")]
    pub total: Money,
}
```

---

## Performance Trade-offs

### Storage Cost

**Thin events**:
- Size: 100-500 bytes
- Cost: $0.023/GB/month (S3)
- 1M events: ~100 MB = $0.002/month

**Fat events**:
- Size: 1-5 KB
- Cost: $0.023/GB/month (S3)
- 1M events: ~1-5 GB = $0.023-0.115/month

**Verdict**: Fat events cost ~$0.10/month per million events. **Worth it** to avoid race conditions and queries.

### Query Performance

**Thin events** (with queries):
```
Event processing: 1ms (event arrival)
+ Projection query: 5-50ms (database query)
= Total: 6-51ms per event
```

**Fat events** (no queries):
```
Event processing: 1ms (event arrival)
= Total: 1ms per event
```

**Verdict**: Fat events are **5-50x faster** (no queries).

### Network Bandwidth

**Thin events**: 100 bytes × 1000 events/sec = 100 KB/sec
**Fat events**: 2 KB × 1000 events/sec = 2 MB/sec

**Verdict**: 20x more bandwidth, but modern networks handle this easily (2 MB/sec << 1 Gbps).

### Overall Trade-off

| Metric | Thin Events | Fat Events | Winner |
|--------|-------------|------------|--------|
| Storage cost | Lower | Higher (~10x) | Thin |
| Query latency | High (5-50ms) | Low (0ms) | **Fat** |
| Race conditions | Common | None | **Fat** |
| Bandwidth | Lower | Higher (20x) | Thin |
| Debugging | Harder | Easier | **Fat** |
| **Verdict** | ❌ | ✅ | **Fat Events** |

**Conclusion**: Fat events cost slightly more in storage/bandwidth but eliminate race conditions and queries. **The trade-off is worth it.**

---

## Testing Event Schemas

### Test 1: Serialization Round-Trip

```rust
#[test]
fn test_event_serialization_round_trip() {
    let event = OrderPlacedEvent {
        version: 1,
        order_id: OrderId::new("order-1"),
        customer_id: CustomerId::new("cust-1"),
        items: vec![test_line_item()],
        total: Money::from_dollars(100),
        timestamp: Utc::now(),
    };

    // Serialize
    let bytes = bincode::serialize(&event).unwrap();

    // Deserialize
    let deserialized: OrderPlacedEvent = bincode::deserialize(&bytes).unwrap();

    // ✅ Should be identical
    assert_eq!(event, deserialized);
}
```

### Test 2: Schema Evolution (Forward Compatibility)

```rust
#[test]
fn test_old_events_can_be_read() {
    // Simulate old event (v1) without discount_code field
    let v1_json = r#"
    {
        "version": 1,
        "order_id": "order-1",
        "customer_id": "cust-1",
        "items": [],
        "total": { "cents": 10000 },
        "timestamp": "2025-01-01T00:00:00Z"
    }
    "#;

    // ✅ Should deserialize successfully
    let event: OrderPlacedEvent = serde_json::from_str(v1_json).unwrap();

    // Defaults should be applied
    assert_eq!(event.version, 1);
    assert_eq!(event.discount_code, None);  // ✅ Default
}
```

### Test 3: Upcasting

```rust
#[test]
fn test_upcast_v1_to_v2() {
    let v1 = OrderPlacedEventV1 {
        order_id: OrderId::new("order-1"),
        items: vec!["prod-1".to_string(), "prod-2".to_string()],
        total: Money::from_dollars(100),
    };

    // Upcast v1 → v2
    let v2: OrderPlacedEventV2 = v1.into();

    // ✅ Conversion should work
    assert_eq!(v2.order_id, OrderId::new("order-1"));
    assert_eq!(v2.items.len(), 2);
    assert_eq!(v2.total, Money::from_dollars(100));
}
```

---

## Best Practices

### ✅ Do This

1. **Use fat events** for critical workflows
2. **Add version field** to all events (day one)
3. **Use past tense** for event names
4. **Include complete data** (no queries needed)
5. **Denormalize lookups** (names, not just IDs)
6. **Pre-calculate values** (totals, tax, etc.)
7. **Make optional fields optional** (`Option<T>`)
8. **Test serialization** (round-trip tests)
9. **Document schema changes** (changelog)
10. **Use bincode** for production (speed and size)

### ❌ Don't Do This

1. ❌ Use thin events for workflows (forces queries)
2. ❌ Forget version field (can't evolve schema)
3. ❌ Use present tense (commands, not events)
4. ❌ Store sensitive data unencrypted
5. ❌ Store large binaries in events
6. ❌ Make breaking changes without upcasting
7. ❌ Use generic names ("UpdatedEvent")
8. ❌ Assume events never change
9. ❌ Skip serialization tests
10. ❌ Use JSON in production (slower, larger)

---

## Event Design Checklist

For each event, verify:

- [ ] **Name**: Past tense, specific, domain language?
- [ ] **Version**: Has version field?
- [ ] **Identifiers**: Has all relevant IDs?
- [ ] **Core data**: Includes what changed?
- [ ] **Complete**: No queries needed to process?
- [ ] **Denormalized**: Includes lookups (names, SKUs)?
- [ ] **Pre-calculated**: Includes totals, tax?
- [ ] **Addresses**: Complete addresses, not IDs?
- [ ] **Metadata**: Has timestamp, correlation ID?
- [ ] **Optional fields**: Uses `Option<T>` for new fields?
- [ ] **Sensitive data**: No unencrypted secrets?
- [ ] **Size**: Under 10 KB (preferably under 5 KB)?
- [ ] **Tested**: Serialization round-trip test exists?
- [ ] **Documented**: Schema changes documented?

---

## Summary

**Fat Event Template**:

```rust
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OrderPlacedEvent {
    // 1. Version
    #[serde(default = "default_version")]
    pub version: u32,

    // 2. Identifiers
    pub order_id: OrderId,
    pub customer_id: CustomerId,

    // 3. Core data (complete)
    pub items: Vec<LineItem>,

    // 4. Pre-calculated values
    pub subtotal: Money,
    pub tax: Money,
    pub total: Money,

    // 5. Complete nested objects
    pub shipping_address: Address,
    pub payment_method: PaymentMethod,

    // 6. Optional fields
    #[serde(default)]
    pub discount_code: Option<String>,

    // 7. Metadata
    pub timestamp: DateTime<Utc>,
    pub correlation_id: Option<Uuid>,
}

fn default_version() -> u32 { 1 }
```

---

## Further Reading

- [Consistency Patterns](./consistency-patterns.md) - When to use fat events
- [Saga Patterns](./saga-patterns.md) - How sagas use events
- [Versioning in an Event Sourced System](https://leanpub.com/esversioning) - Greg Young's book
- [Event Sourcing Basics](https://martinfowler.com/eaaDev/EventSourcing.html) - Martin Fowler

---

**Last Updated**: 2025-01-07
**Status**: ✅ Production Ready
