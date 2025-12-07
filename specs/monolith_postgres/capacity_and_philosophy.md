# Capacity Planning and Architectural Philosophy

> **Audience**: Technical leaders, architects, and developers evaluating this architecture
>
> This document addresses the question: "When is a PostgreSQL monolith enough?"
> The answer, for most businesses, is "almost always."

---

## 1. The Core Thesis

**Most "scale problems" are actually organization problems or architecture problems in disguise.**

When companies say they need microservices or distributed systems for scale, they usually mean something else entirely:

| What they say | What they actually mean | Real solution |
|---------------|------------------------|---------------|
| "We need to scale" | "Our teams block each other" | Bounded contexts (org design) |
| "Our monolith can't handle load" | "Our monolith is a big ball of mud" | DDD + clean architecture |
| "We need independent deployments" | "We want team autonomy" | Team-per-context ownership |
| "We have performance issues" | "We have N+1 queries and missing indexes" | Basic optimization |

The skill isn't building distributed systems. **The skill is designing boundaries so you don't need them.**

---

## 2. PostgreSQL Capacity Reality

### 2.1 Hardware Baseline

A "good modern server" in today's data centers:

```
CPU:        16-32 cores (AMD EPYC / Intel Xeon)
RAM:        128-256 GB
Storage:    NVMe SSD (500k+ IOPS, 3+ GB/s sequential)
Network:    10 Gbps
Cost:       ~$1,000-2,000/month (cloud) or equivalent bare metal
```

### 2.2 Write Throughput (Event Sourcing)

| Scenario | TPS | Notes |
|----------|-----|-------|
| Simple INSERT (single event) | 15,000-30,000 | `synchronous_commit=on` (safe) |
| Simple INSERT (relaxed durability) | 40,000-80,000 | `synchronous_commit=off` |
| With projection trigger | 8,000-15,000 | INSERT fires trigger + projection UPDATE |
| Batched INSERTs (100/batch) | 100,000-300,000 rows/sec | Amortizes commit overhead |

**For our architecture** (global log + typed context table + projection trigger):

```
Realistic sustained:    5,000-10,000 events/second
Peak bursts:           15,000-20,000 events/second
```

### 2.3 Read Throughput (Projections)

| Query Type | QPS | Notes |
|------------|-----|-------|
| Point lookup by PK | 100,000-200,000 | Data in shared_buffers |
| Point lookup (cache miss) | 30,000-50,000 | NVMe latency ~100μs |
| Index scan (small result) | 50,000-100,000 | Depends on selectivity |
| Complex join | 1,000-10,000 | Highly variable |

### 2.4 Connection Handling

| Configuration | PostgreSQL Connections | Client Capacity |
|---------------|------------------------|-----------------|
| Direct PostgreSQL | 200-500 | Limited by RAM (~10MB/conn) |
| With PgBouncer | 50-100 to PostgreSQL | 5,000-10,000 clients |

### 2.5 Storage Growth

```
Event size (average):       500 bytes - 2 KB (with JSONB payload)
Events/day at 1,000 TPS:    ~86 million events
Storage/day (uncompressed): ~50-150 GB
Storage/day (compressed):   ~15-50 GB (TOAST compression on JSONB)
Storage/year at 2,000 TPS:  ~2-5 TB (before archiving old partitions)
```

---

## 3. The Math: What Businesses Actually Need

### 3.1 Enterprise Internal Systems (10,000 employees)

```
Active users:           10,000
Actions per day:        100 per user (generous estimate)
Total actions/day:      1,000,000
Average TPS:            ~12
Peak TPS:               ~100-200

PostgreSQL capacity:    5,000+ TPS
Headroom:               25-50x over peak
```

**Verdict**: A single PostgreSQL handles this trivially.

### 3.2 Large E-commerce (Top 100 Retailer)

```
Orders per day:         500,000 (very successful retailer)
Average TPS:            ~6
Peak (Black Friday):    ~500 TPS

PostgreSQL capacity:    5,000+ TPS
Headroom:               10x even at absolute peak
```

**Verdict**: Comfortable headroom even during peak events.

### 3.3 Context: Amazon Scale

```
Amazon average:         ~66 orders/second
Amazon Prime Day peak:  ~10,000+ orders/second (estimated)
```

A single PostgreSQL monolith could handle **30x average Amazon order volume**.

But Amazon doesn't run one monolith—they have hundreds of bounded contexts. Each context individually handles a fraction of this load.

### 3.4 The Bounded Context Multiplier

When you decompose by bounded context, load distributes naturally:

```
┌─────────────────────────────────────────────────────────────────┐
│                    LARGE E-COMMERCE                              │
│                                                                  │
│   Orders Context:      500 TPS peak  →  One PostgreSQL          │
│   Inventory Context:   200 TPS peak  →  One PostgreSQL          │
│   Payments Context:    500 TPS peak  →  One PostgreSQL          │
│   Shipping Context:    100 TPS peak  →  One PostgreSQL          │
│   Customers Context:    50 TPS peak  →  One PostgreSQL          │
│   Catalog Context:     Read-heavy    →  One PostgreSQL + replicas│
│                                                                  │
│   Each context: TRIVIAL load for PostgreSQL                     │
│   Each team: Full ownership of their database                   │
│   Integration: Async events between contexts                    │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

**The insight**: It's not about technology being unable to scale. It's about finding boundaries so each piece doesn't NEED to scale beyond what simple technology handles easily.

---

## 4. Natural Sharding in Business Domains

Most businesses have natural shard boundaries built into their domain:

| Business Type | Natural Shard | Data Independence |
|---------------|---------------|-------------------|
| **Multi-tenant SaaS** | Customer/Tenant | 100% isolated |
| **Regional operations** | Geography (EU/US/APAC) | Mostly isolated |
| **Franchise/Retail** | Store/Location | Highly isolated |
| **Healthcare** | Hospital/Clinic | Legally isolated (HIPAA) |
| **Financial services** | Account holder | Isolated by regulation |
| **Enterprise IT** | Department/Business unit | Organizationally isolated |
| **Education** | School/University | Administratively isolated |

### 4.1 Designing for Natural Shards

Build tenant isolation from day one:

```sql
-- Every table has tenant isolation built in
CREATE TABLE sales.orders (
    tenant_id   UUID NOT NULL,
    order_id    UUID NOT NULL,
    -- ... other columns
    PRIMARY KEY (tenant_id, order_id)
);

-- Indexes lead with tenant_id for efficient filtering
CREATE INDEX idx_orders_tenant_status
    ON sales.orders (tenant_id, status, created_at DESC);

-- Row-level security enforces isolation automatically
ALTER TABLE sales.orders ENABLE ROW LEVEL SECURITY;

CREATE POLICY tenant_isolation ON sales.orders
    USING (tenant_id = current_setting('app.tenant_id')::uuid);
```

### 4.2 The Scaling Escape Hatch

With tenant-aware design, scaling is operational, not architectural:

```
Year 1:  All tenants in one PostgreSQL (simple)
Year 2:  Add read replicas for reporting (still simple)
Year 3:  Largest tenant gets dedicated PostgreSQL (same code, same schema)
Year 4:  Shard by region if needed (EU data stays in EU)

No code changes. No architecture changes. Just operational scaling.
```

---

## 5. The Organizational Insight

DDD's bounded contexts are as much about **people** as they are about **code**.

### 5.1 Traditional Approach (Problems)

```
One database           →  One "DBA team"        →  Bottleneck
Shared codebase        →  Merge conflicts       →  Slow releases
Shared schema          →  Migration coordination →  Fear of change
Coupled deployments    →  Release trains        →  Slow iteration
```

### 5.2 Context-Per-Team Approach (Solutions)

```
Context A: Team A owns database, code, deployment, schema
Context B: Team B owns database, code, deployment, schema
Context C: Team C owns database, code, deployment, schema

Integration: Async events with explicit contracts

Result: Teams move independently at their own pace
```

### 5.3 Conway's Law Working FOR You

> "Organizations which design systems are constrained to produce designs
> which are copies of the communication structures of these organizations."
> — Melvin Conway

Instead of fighting this, embrace it:

```
┌─────────────────────────────────────────────────────────────────┐
│                    ORGANIZATIONAL ALIGNMENT                      │
│                                                                  │
│   Sales Team          →  Sales Context (owns PostgreSQL)        │
│   Inventory Team      →  Inventory Context (owns PostgreSQL)    │
│   Shipping Team       →  Shipping Context (owns PostgreSQL)     │
│   Platform Team       →  Integration infrastructure             │
│                                                                  │
│   Each team:                                                     │
│   ├── Owns their bounded context end-to-end                     │
│   ├── Deploys independently                                     │
│   ├── Chooses their own release cadence                         │
│   ├── Manages their own database schema                         │
│   └── Communicates via published events (contracts)             │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

---

## 6. The Industry's Dirty Secret

Many famous "scale" stories are actually about **organizational independence**, not technical scale:

| Company | What they say | The reality |
|---------|---------------|-------------|
| Netflix | "We need microservices for scale" | Organizational independence for 2,000+ engineers |
| Amazon | "We need distributed systems" | Team autonomy ("two-pizza teams") |
| Google | "We built custom infrastructure" | They genuinely DO have unique scale requirements |
| Typical startup | "We need Kubernetes and microservices" | Often resume-driven development |

### 6.1 Counter-Examples

**Shopify**: Runs one of the world's largest e-commerce platforms on a (sharded) Rails monolith with MySQL. Handles Black Friday traffic for millions of merchants.

**Stack Overflow**: Serves 1.3 billion page views/month on 9 web servers and 4 SQL Servers. Their architecture is famously "boring."

**Basecamp (37signals)**: Runs a successful SaaS business on a small number of servers with PostgreSQL. Wrote "Getting Real" about simplicity.

**Telegram**: Handles 700 billion messages/month. Core infrastructure is surprisingly simple (custom MTProto, but conceptually straightforward).

### 6.2 The Pattern

Successful companies that "scale" often:
1. Start with simple technology
2. Optimize the simple technology until it breaks
3. Only then add complexity, and only where needed
4. Keep most of the system simple

They don't start with distributed systems. They earn them.

---

## 7. When You Actually Need More

This architecture genuinely won't handle:

### 7.1 Global Latency Requirements

```
User in Sydney → Server in Virginia = 200ms+ round trip
Physics wins. No amount of optimization helps.

Examples:
├── Global real-time collaboration (Google Docs)
├── Global multiplayer games
├── Global chat applications
└── Any "instant" interaction across continents

Solution: Geo-distributed data (CockroachDB, Spanner, or regional PostgreSQL instances)
```

### 7.2 Extreme Per-Aggregate Write Throughput

```
Single aggregate receiving >10,000 writes/second

Examples:
├── Global "like" counter (Instagram celebrity post)
├── Real-time bidding (ad tech)
├── Live event voting/polling
└── Viral content interactions

Solution: CRDT/eventual consistency, not stronger infrastructure
```

### 7.3 High-Frequency Trading / Microsecond Latency

```
Requirements: <100 microsecond latency
PostgreSQL: ~1-10 millisecond latency (1000-10000x too slow)

Examples:
├── Stock exchanges
├── High-frequency trading
└── Real-time financial arbitrage

Solution: Custom in-memory systems, FPGAs, kernel bypass networking
```

### 7.4 Massive IoT/Telemetry Ingestion

```
Millions of sensors reporting every second
Time-series data with specific query patterns

Examples:
├── Industrial IoT platforms
├── Connected vehicle telemetry
├── Infrastructure monitoring at scale
└── Smart city sensor networks

Solution: Purpose-built time-series databases (TimescaleDB, InfluxDB, QuestDB)
```

### 7.5 Extreme Availability Requirements (Five 9s)

```
99.999% uptime = 5 minutes downtime per year
Requires active-active multi-region deployment

PostgreSQL CAN do this (Patroni + Citus + careful design)
But it's complex and requires expertise

Most businesses don't actually need five 9s
(Do the math: what does 1 hour of downtime actually cost?)
```

### 7.6 The Key Observation

Even companies with these problems usually have them in **ONE context**, not all of them:

```
Large Company:
├── Orders Context:      Normal PostgreSQL (99% of the time)
├── Inventory Context:   Normal PostgreSQL
├── Payments Context:    Normal PostgreSQL
├── Analytics Context:   Maybe needs special handling
└── Viral Content:       Definitely needs special handling ← The 1%

Don't let the 1% dictate architecture for the 99%.
```

---

## 8. Architecture Decision Framework

### 8.1 Start Here (Default)

```
┌─────────────────────────────────────────────────────────────────┐
│                    DEFAULT ARCHITECTURE                          │
│                                                                  │
│   One PostgreSQL per bounded context                            │
│   ├── Event sourcing (append-only events)                       │
│   ├── Projections (materialized read models)                    │
│   ├── pg-gateway thin shell (protocol translation)              │
│   └── Async integration events between contexts                 │
│                                                                  │
│   Scale vertically first:                                        │
│   ├── Bigger server (more cores, RAM, faster NVMe)              │
│   ├── Connection pooling (PgBouncer)                            │
│   ├── Read replicas for read-heavy contexts                     │
│   └── Table partitioning for large event logs                   │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

### 8.2 Decision Tree

```
Is your load > 5,000 writes/second to a SINGLE aggregate?
├── No  → PostgreSQL is fine
└── Yes → Is it a counter/vote/like scenario?
          ├── Yes → Use CRDT or eventual consistency
          └── No  → Consider sharding that specific context

Do you need < 50ms latency for users on multiple continents?
├── No  → Single region PostgreSQL is fine
└── Yes → Consider geo-distributed database for that context

Do you need 99.999% availability?
├── No  → Standard PostgreSQL HA (Patroni) is fine
└── Yes → Consider multi-region active-active (complex)

Is your team > 50 engineers working on the same context?
├── No  → Single codebase is fine
└── Yes → Consider splitting the context (it's probably too big)
```

### 8.3 Warning Signs You're Over-Engineering

- Adding Kubernetes before you have 10 services
- Adding Kafka before you have 1,000 events/second
- Adding microservices before you have 10 engineers
- Adding distributed tracing before you have distributed systems
- Choosing "scalable" over "simple" without hitting limits
- Optimizing for problems you don't have yet

### 8.4 Warning Signs You Need to Scale

| Metric | Investigate | Act Now |
|--------|-------------|---------|
| Query latency (p95) | > 50ms | > 200ms |
| Write latency (p95) | > 20ms | > 100ms |
| CPU utilization | > 70% sustained | > 90% |
| Connection pool waits | > 50ms | > 200ms |
| Replication lag | > 5 seconds | > 30 seconds |
| Storage growth | 80% capacity | 90% capacity |

---

## 9. What This Architecture Handles Well

```
┌─────────────────────────────────────────────────────────────────┐
│                                                                  │
│   SWEET SPOT: 95%+ OF BUSINESS APPLICATIONS                     │
│   ═══════════════════════════════════════════                   │
│                                                                  │
│   ✓ Startups → Scale-ups → Enterprises                         │
│   ✓ Multi-tenant SaaS products                                  │
│   ✓ E-commerce platforms (even large ones)                      │
│   ✓ Internal enterprise systems                                 │
│   ✓ Healthcare systems (HIPAA-compliant)                        │
│   ✓ Financial services (audit trails, compliance)              │
│   ✓ Insurance platforms                                         │
│   ✓ B2B marketplaces and platforms                              │
│   ✓ Content management systems                                  │
│   ✓ ERP/CRM systems                                             │
│   ✓ Booking and reservation systems                             │
│   ✓ Workflow and process management                             │
│   ✓ Education platforms                                         │
│   ✓ Government/civic applications                               │
│                                                                  │
│   NEEDS SPECIAL CONSIDERATION: ~5%                               │
│   ═══════════════════════════════                                │
│                                                                  │
│   ⚠ Global real-time collaboration                              │
│   ⚠ Social media "firehose" (viral content)                     │
│   ⚠ High-frequency trading                                      │
│   ⚠ Massive IoT ingestion                                       │
│   ⚠ Real-time multiplayer games                                 │
│   ⚠ Ad tech / real-time bidding                                 │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

---

## 10. Summary

### The Philosophy

1. **Simple technology, sophisticated design**: PostgreSQL is boring. DDD bounded contexts are sophisticated. Combine them.

2. **Scale the organization, not the technology**: Most "scale" problems are solved by team boundaries, not distributed systems.

3. **Earn complexity**: Start simple. Add complexity only when you hit real limits, not imagined ones.

4. **Design boundaries, not infrastructure**: The skill is finding natural seams in the domain so each piece stays small.

### The Numbers

- **5,000+ TPS** per PostgreSQL instance (writes with projections)
- **100,000+ QPS** for reads from projections
- **Years of growth** before needing to scale out
- **95%+ of businesses** never need more than this

### The Approach

```
Year 1:    One PostgreSQL, prove the business
Year 2:    Add read replicas if read-heavy
Year 3:    Split contexts if teams need independence
Year 4+:   Shard large tenants if genuinely needed

At each stage: Same architecture, same code, same patterns
Only the deployment topology changes
```

### The Bottom Line

**Don't build for Google scale unless you have Google problems.**

Most businesses—including large enterprises with thousands of employees—are well-served by:
- Well-designed bounded contexts (DDD)
- One PostgreSQL per context
- Async event integration
- Natural tenant/region sharding

The vast majority of applications will never outgrow this architecture. And if they do, the bounded context design means you only need to scale the specific context that needs it—not rebuild everything.

**Choose boring technology. Design sophisticated boundaries. Ship products.**
