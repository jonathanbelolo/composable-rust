# Global Deployment Platforms Guide

**Purpose**: Choose the right platform for globally distributed, regionally optimized AI agent deployment

**Use Case**: Production agent that needs to serve users in Tokyo, Paris, San Francisco, and New York with low latency, starting in Paris and scaling incrementally.

---

## Table of Contents

1. [Executive Summary](#executive-summary)
2. [Platform Spectrum](#platform-spectrum)
3. [Detailed Platform Analysis](#detailed-platform-analysis)
4. [Cost Comparison](#cost-comparison)
5. [Regional Deployment Strategies](#regional-deployment-strategies)
6. [Recommendations](#recommendations)

---

## Executive Summary

**TL;DR for AI Agent Deployment**:

| If You Want... | Choose | Why |
|----------------|--------|-----|
| **Best for your use case** | **Fly.io** | Global deployment, regional databases, simple scaling, Rust-optimized |
| Maximum control | AWS EKS + RDS Multi-Region | Most powerful, most complex |
| Best price/performance | GCP GKE Autopilot | Auto-optimizes, good global network |
| Easiest to start | Railway or Render | Deploy in minutes, expand later |
| Edge proximity | Cloudflare Workers + D1 | Ultra-low latency, but limited stateful options |

**Recommended Path for Your Agent**:
1. **Start**: Fly.io (1 region: Paris)
2. **Expand**: Add Tokyo, SF, NYC regions on Fly.io
3. **Scale**: Stay on Fly.io or migrate to GKE if you hit limits

**Why Kubernetes is "SOTA"** (but not always the answer):
- ✅ Industry standard, portable across clouds
- ✅ Handles complex stateful workloads well
- ✅ Excellent for multi-region orchestration
- ❌ High complexity, steep learning curve
- ❌ Expensive for small deployments
- ❌ Overkill for early-stage products

---

## Platform Spectrum

```
Low-Level ←──────────────────────────────────────────→ High-Level
(Max Control, Max Complexity)                (Min Control, Min Complexity)

┌─────────────┬──────────────┬─────────────┬─────────────┬──────────────┐
│   Bare      │  Kubernetes  │  Container  │   Modern    │     Edge     │
│   Metal     │  (Managed)   │    PaaS     │    PaaS     │  Computing   │
└─────────────┴──────────────┴─────────────┴─────────────┴──────────────┘
│             │              │             │             │              │
│ EC2         │ GKE          │ Cloud Run   │ Fly.io      │ Cloudflare   │
│ Bare Metal  │ EKS          │ ECS Fargate │ Railway     │ Deno Deploy  │
│ Hetzner     │ AKS          │ App Engine  │ Render      │ Lambda@Edge  │
│             │ DigitalOcean │             │ Heroku      │              │

Cost:      $$          $$$         $$           $$$            $$
Complexity: ████████    ██████      ███          █              ██
Control:    ████████    ██████      ████         ██             ███
Portability:  ████      ████████    ████         ███            █
```

---

## Detailed Platform Analysis

### 1. Bare Metal / VMs (DIY Infrastructure)

**Examples**: AWS EC2, Azure VMs, GCP Compute Engine, Hetzner, OVH

**How It Works**:
- You rent virtual machines (or physical servers)
- Install everything yourself (Docker, databases, load balancers)
- Configure networking, security, monitoring
- Fully manual horizontal scaling

**Pros**:
- ✅ Maximum control over every aspect
- ✅ Cheapest if you optimize well
- ✅ No platform lock-in
- ✅ Good for specific compliance requirements

**Cons**:
- ❌ Highest operational burden (you're the ops team)
- ❌ Manual scaling and load balancing
- ❌ Harder to do multi-region
- ❌ Security is your responsibility
- ❌ Steep learning curve for production-grade setup

**Cost (Monthly Estimate for 3 regions)**:
```
Paris (development):
  - 2x t3.medium (2 vCPU, 4GB) @ $60/mo = $120
  - PostgreSQL RDS db.t4g.medium @ $80/mo = $80
  - Redis ElastiCache t3.micro @ $15/mo = $15
  - NAT Gateway @ $32/mo = $32
  - Data transfer @ ~$50/mo = $50
  Subtotal: ~$300/mo

Scale to 3 regions (Tokyo, SF, NYC): ~$900/mo
Plus: Time spent on ops (your salary × hours)
```

**Best For**:
- Companies with dedicated DevOps teams
- Cost optimization at massive scale (1000+ servers)
- Specific compliance needs (healthcare, finance)

**NOT Recommended For**:
- Early-stage products
- Small teams without DevOps expertise
- Rapid iteration

---

### 2. Kubernetes (Managed)

**Examples**:
- **AWS EKS** (Elastic Kubernetes Service)
- **GCP GKE** (Google Kubernetes Engine) - **Best K8s experience**
- **Azure AKS** (Azure Kubernetes Service)
- **DigitalOcean Kubernetes**
- **Linode Kubernetes Engine**

**How It Works**:
- Cloud provider manages the Kubernetes control plane
- You deploy containers using K8s manifests (we've already built these!)
- Auto-scaling, health checks, rolling updates built-in
- Multi-region requires separate clusters + global load balancing

**Pros**:
- ✅ Industry standard - skills are portable
- ✅ Excellent for complex stateful workloads
- ✅ Strong ecosystem (Helm, Operators, monitoring tools)
- ✅ Works well with our existing K8s manifests
- ✅ Good for hybrid cloud / multi-cloud
- ✅ Mature autoscaling (HPA, VPA, cluster autoscaler)

**Cons**:
- ❌ Steep learning curve (weeks to months)
- ❌ Expensive for small deployments ($72/mo minimum per cluster)
- ❌ Complexity: you still manage worker nodes, networking, storage
- ❌ Multi-region = multiple clusters = higher cost
- ❌ Over-engineering for simple use cases

**Cost (Monthly Estimate)**:

**GKE (Best K8s experience)**:
```
Paris (development):
  - GKE Autopilot cluster (managed nodes) @ $0
  - Workload resources:
    - Production agent: 2 pods × (0.5 CPU, 1GB RAM) = 1 CPU, 2GB
    - PostgreSQL StatefulSet: 1 CPU, 2GB
    - Redis: 0.5 CPU, 1GB
    - Redpanda: 1 CPU, 2GB
  Total: ~3.5 CPU, 7GB RAM

  GKE Autopilot cost:
  - CPU: 3.5 × $30/vCPU/mo = $105
  - Memory: 7GB × $3.30/GB/mo = $23
  - Persistent disks: 50GB × $0.17/GB/mo = $8.50
  Subtotal: ~$140/mo

Scale to 3 regions: ~$420/mo
Add managed CloudSQL (better than self-hosted): +$200/mo
Total: ~$620/mo
```

**EKS (AWS)**:
```
Paris (development):
  - EKS control plane: $72/mo
  - 3x t3.medium worker nodes: 3 × $60/mo = $180
  - EBS volumes: 50GB × $0.10/GB/mo = $5
  - Load balancer: $20/mo
  Subtotal: ~$280/mo

Scale to 3 regions: ~$840/mo
Add RDS Multi-AZ: +$160/mo
Total: ~$1,000/mo
```

**Why GKE > EKS > AKS**:
1. **GKE Autopilot**: Google manages nodes, auto-optimizes, pay only for pods
2. **GKE Networking**: Best global network (Google's backbone)
3. **EKS**: Good AWS integration, but more expensive, more manual
4. **AKS**: Decent, but Azure network not as good globally

**Best For**:
- Teams with K8s expertise
- Complex microservices architectures
- Multi-cloud requirements
- Enterprise deployments

**Good For Your Use Case If**:
- You want maximum control
- You plan to hire DevOps engineers
- You're building a multi-service platform

---

### 3. Container PaaS (Managed Containers)

**Examples**:
- **Google Cloud Run** (best for stateless, serverless containers)
- **AWS ECS Fargate** (AWS-native, no K8s)
- **Azure Container Instances**
- **AWS App Runner** (simplest AWS option)

**How It Works**:
- Deploy containers without managing infrastructure
- Platform handles scaling, load balancing, health checks
- Pay per request (Cloud Run) or per container-second (Fargate)

**Pros**:
- ✅ Simpler than Kubernetes
- ✅ Good autoscaling (including to zero for Cloud Run)
- ✅ No cluster management
- ✅ Fast deployment
- ✅ Built-in CI/CD

**Cons**:
- ❌ Less portable (tied to cloud provider)
- ❌ Limited for stateful workloads
- ❌ Harder to do complex networking
- ❌ Regional deployment requires duplication

**Cost (Monthly Estimate)**:

**Google Cloud Run (Best option in this category)**:
```
Paris (development):
  Production agent:
  - 100,000 requests/mo, avg 500ms response
  - 0.5 vCPU, 1GB RAM per instance
  - Auto-scales 0-10 instances

  Cost:
  - CPU time: 100k × 0.5s × $0.00002400/vCPU-sec = $1.20
  - Memory: 100k × 0.5s × $0.00000250/GB-sec = $0.13
  - Requests: 100k × $0.40/million = $0.04
  Subtotal: ~$2/mo (!!!)

  Add CloudSQL (PostgreSQL): $80/mo
  Add Memorystore (Redis): $50/mo
  Total: ~$132/mo

Scale to 3 regions: ~$400/mo (mostly databases)
```

**Limitations for Your Use Case**:
- Stateful workloads (databases) need separate managed services
- WebSocket support is limited (Cloud Run has timeouts)
- Not ideal for long-running connections
- Redpanda/Kafka requires separate deployment

**Best For**:
- Stateless HTTP APIs
- Event-driven workloads
- Bursty traffic patterns
- Microservices with managed databases

**Verdict for AI Agent**:
- ⚠️ **Partial fit** - Good for the agent API, but need separate database solutions
- ⚠️ WebSocket limitations might be problematic

---

### 4. Modern PaaS (Developer-First Platforms)

**Examples**:
- **Fly.io** ⭐ **HIGHLY RECOMMENDED FOR YOUR USE CASE**
- **Railway**
- **Render**
- **Heroku** (legacy, expensive)

#### 4a. Fly.io (⭐ Best for Your Use Case)

**Why Fly.io is Perfect for Global AI Agents**:

**How It Works**:
- Deploy Docker containers to edge locations worldwide
- Automatic global load balancing (Anycast routing)
- **Fly Postgres** - distributed PostgreSQL with read replicas
- **Fly Redis** - Redis in every region
- **Fly Machines** - fast, lightweight VMs for containers

**Unique Features for Global Deployment**:
1. **True Multi-Region Made Easy**:
   ```bash
   # Deploy to Paris
   fly deploy --region cdg

   # Add Tokyo
   fly regions add nrt

   # Add San Francisco
   fly regions add sjc

   # Add New York
   fly regions add ewr

   # Fly automatically routes users to nearest region!
   ```

2. **Regional Databases**:
   - Primary PostgreSQL in Paris
   - Read replicas in Tokyo, SF, NYC (auto-sync)
   - Redis in each region (can use as cache or session store)

3. **Built-in Features**:
   - Auto TLS certificates (Let's Encrypt)
   - Built-in load balancing
   - Health checks and auto-restart
   - IPv4 + IPv6
   - Private networking (WireGuard)

**Pros**:
- ✅ **PERFECT for global deployment** - designed for this use case
- ✅ Incremental scaling (start in 1 region, add more anytime)
- ✅ Excellent Rust support (Fly loves Rust!)
- ✅ Simple CLI, great DX
- ✅ WebSocket support (no timeouts)
- ✅ Built-in metrics and logging
- ✅ Fast cold starts (~200ms)
- ✅ Regional databases included

**Cons**:
- ❌ Proprietary platform (some lock-in)
- ❌ Smaller company (vs AWS/GCP - higher risk)
- ❌ No Redpanda as managed service (would need to self-host)
- ❌ Can get expensive at very high scale
- ❌ Less mature than AWS/GCP

**Cost (Monthly Estimate)**:

```
Paris Only (Development):
  Production agent:
  - 1x shared-cpu-1x (1 vCPU, 256MB) @ $3/mo = $3
  - Or 1x dedicated-cpu-1x (1 vCPU, 2GB) @ $23/mo = $23

  Fly Postgres (managed):
  - 1x postgres-dev (1 vCPU, 256MB) @ $0/mo = $0 (free tier!)
  - Or 1x postgres-flex-1x (1 vCPU, 2GB) @ $15/mo = $15

  Fly Redis:
  - 1x redis-fly (256MB) @ $0/mo = $0 (free tier!)
  - Or 1x redis-1g (1GB) @ $10/mo = $10

  IPv4 address: $2/mo

  Total (free tier): $5/mo (!!)
  Total (production): ~$50/mo

Global (Paris + Tokyo + SF + NYC):
  Production agent:
  - 4x dedicated-cpu-1x @ $23/mo = $92

  Fly Postgres:
  - Primary in Paris: $15/mo
  - 3x read replicas: 3 × $15/mo = $45

  Fly Redis (per region):
  - 4x redis-1g @ $10/mo = $40

  4x IPv4 addresses: 4 × $2 = $8

  Total: ~$200/mo for global deployment! 🎉
```

**Fly.io Deployment Example**:
```toml
# fly.toml
app = "production-agent"

[build]
  dockerfile = "Dockerfile"

[env]
  RUST_LOG = "info"

[[services]]
  internal_port = 8080
  protocol = "tcp"

  [[services.ports]]
    handlers = ["http"]
    port = 80

  [[services.ports]]
    handlers = ["tls", "http"]
    port = 443

[metrics]
  port = 9090
  path = "/metrics"

# Deploy to multiple regions
[regions]
  cdg = "Paris"
  nrt = "Tokyo"
  sjc = "San Francisco"
  ewr = "New York"
```

**Regional Latency with Fly.io**:
- User in Tokyo → Tokyo instance: ~10-30ms
- User in Paris → Paris instance: ~10-30ms
- User in SF → SF instance: ~10-30ms
- Automatic routing based on user location!

**Recommendation**: ⭐ **START HERE**
- Deploy to Paris first
- Test with real users
- Add regions incrementally
- Stay on Fly.io unless you outgrow it (unlikely for most products)

---

#### 4b. Railway

**How It Works**:
- GitHub integration (auto-deploy on push)
- Simple dashboard, no complex config
- Managed PostgreSQL, Redis included

**Pros**:
- ✅ Extremely simple setup
- ✅ Great for prototyping
- ✅ Nice UI/UX

**Cons**:
- ❌ **No multi-region support** (US only)
- ❌ More expensive than Fly.io at scale
- ❌ Less control

**Cost**: ~$20/mo for small app + $10/mo for Postgres = $30/mo

**Verdict**: ❌ Not suitable for global deployment

---

#### 4c. Render

**How It Works**:
- Similar to Railway
- Auto-deploy from Git
- Managed services included

**Pros**:
- ✅ Simple setup
- ✅ Free tier available
- ✅ Multiple regions (US, EU, Singapore)

**Cons**:
- ❌ Regional deployment not as seamless as Fly.io
- ❌ More expensive
- ❌ Limited global optimization

**Cost**: ~$25/mo for app + $15/mo for Postgres = $40/mo per region

**Verdict**: ⚠️ Possible, but Fly.io is better for global deployment

---

### 5. Edge Computing (Global by Default)

**Examples**:
- **Cloudflare Workers** + Cloudflare D1/Durable Objects
- **Deno Deploy**
- **AWS Lambda@Edge**
- **Vercel Edge Functions**

**How It Works**:
- Code runs in edge locations worldwide (100+ locations)
- Automatically routed to nearest edge
- Serverless, pay per request
- Ultra-low latency (<50ms globally)

**Cloudflare Workers for AI Agents**:

**Pros**:
- ✅ **Lowest latency possible** (runs in 300+ cities)
- ✅ Automatic global distribution
- ✅ Very cheap at scale
- ✅ Built-in DDoS protection
- ✅ Can call Anthropic API directly

**Cons**:
- ❌ **Stateful workloads are hard** (limited database options)
- ❌ Different programming model (V8 isolates, not containers)
- ❌ **No native Rust support** (only JavaScript/WASM)
- ❌ Cold start issues for WASM
- ❌ Limited compute time (30-50 seconds max)

**Cost**:
```
Cloudflare Workers:
  - Free tier: 100,000 requests/day
  - Paid: $5/mo + $0.50 per million requests

For 10M requests/mo: $5 + $5 = $10/mo

Cloudflare D1 (SQL database):
  - Free tier: 5GB storage, 5M reads/day
  - Paid: $5/mo + usage
```

**Can You Deploy Rust on Cloudflare Workers?**:
- ✅ Yes, via WebAssembly (WASM)
- ⚠️ Requires compiling to WASM target
- ⚠️ Not all Rust libraries work in WASM
- ⚠️ Cold starts can be slow for large WASM binaries

**Example (Simplified)**:
```rust
// Would need to compile to WASM
#[wasm_bindgen]
pub async fn handle_request(req: Request) -> Response {
    // Call Anthropic API
    // Limited to 30 seconds execution time
}
```

**Verdict for Your Use Case**:
- ❌ **Not recommended** - too limiting for stateful AI agent
- ❌ Long-running conversations would hit time limits
- ❌ Database/Redis/Redpanda integration is problematic
- ✅ Could use as CDN/API gateway in front of main deployment

---

## Cost Comparison Summary

**Monthly costs for global deployment (Paris + Tokyo + SF + NYC)**:

| Platform | Small Scale | Medium Scale | Large Scale | Notes |
|----------|-------------|--------------|-------------|-------|
| **Fly.io** ⭐ | **$200** | **$500** | **$2,000** | Best value for global |
| GKE Autopilot | $620 | $1,500 | $5,000+ | Scales well |
| AWS EKS | $1,000 | $2,500 | $8,000+ | Most expensive |
| Railway | N/A | N/A | N/A | US only |
| Render | $160/region | $400/region | $1,200/region | Per region pricing |
| Cloud Run + SQL | $400 | $800 | $2,000 | Good for stateless |
| Cloudflare Workers | $10 | $50 | $200 | Cheapest but limited |

**"Small Scale"**: 10K requests/day, 2 GB database, minimal compute
**"Medium Scale"**: 100K requests/day, 20 GB database, moderate compute
**"Large Scale"**: 1M+ requests/day, 100+ GB database, high compute

---

## Regional Deployment Strategies

### Strategy 1: Start Small, Scale Incrementally (RECOMMENDED)

**Phase 1: Single Region (Paris)**
```
Deploy to: Paris (cdg)
Cost: ~$50/mo on Fly.io
Users: French/EU market
Latency:
  - Paris: 10-20ms
  - NYC: 100-120ms
  - Tokyo: 200-300ms ❌ Too slow for Tokyo users
```

**Phase 2: Add Critical Regions**
```
Deploy to: Paris + Tokyo
Cost: ~$100/mo
Users: EU + Asia
Latency:
  - Paris: 10-20ms ✅
  - Tokyo: 10-20ms ✅
  - NYC: 100-120ms ⚠️ Acceptable
  - SF: 80-100ms ⚠️ Acceptable
```

**Phase 3: Full Global**
```
Deploy to: Paris + Tokyo + SF/NYC
Cost: ~$200/mo
Users: Global
Latency: <50ms everywhere ✅
```

### Strategy 2: Regional Database Replication

**Option A: Primary + Read Replicas (Fly.io)**
```
Primary Database: Paris (writes)
Read Replicas: Tokyo, SF, NYC (reads only)

Writes: Go to Paris (add latency)
Reads: Local (fast)

Good for: Read-heavy workloads (most AI agents)
```

**Option B: Multi-Master (Complex)**
```
Active databases in all regions
Conflict resolution required
Much more complex

Good for: Write-heavy workloads
Not recommended initially
```

### Strategy 3: Regional Data Sovereignty

**For compliance (GDPR, data residency)**:
```
EU users → EU database (Paris)
US users → US database (Virginia)
Asia users → Asia database (Tokyo)

No cross-region data transfer
Meets strict compliance requirements
```

---

## Recommendations

### For Your AI Agent (Production-Ready Path)

**🏆 Recommended: Fly.io**

**Why**:
1. ✅ Built for global deployment (your #1 requirement)
2. ✅ Regional databases included
3. ✅ Incremental scaling (Paris → Global)
4. ✅ Excellent Rust support
5. ✅ Best price/performance for your use case
6. ✅ Simple deployment (we already have Docker images)
7. ✅ WebSocket support (long-running connections OK)

**Deployment Plan**:
```bash
# Week 1: Deploy to Paris
fly launch --region cdg
fly postgres create production-agent-db
fly redis create production-agent-cache

# Week 2-4: Test with French users
# Monitor latency, fix bugs

# Month 2: Expand to Tokyo
fly regions add nrt
fly postgres attach --region nrt (creates read replica)
fly redis create production-agent-cache-tokyo

# Month 3: Add North America
fly regions add sjc ewr
# Replicas auto-created

# You're now global! 🌍
```

**Cost Trajectory**:
- Month 1 (Paris): $50/mo
- Month 2 (Paris + Tokyo): $100/mo
- Month 3 (Global): $200/mo
- Scale up as needed

---

### Alternative: GKE Autopilot (If You Need K8s)

**When to choose GKE instead**:
- ✅ You have K8s expertise (or want to build it)
- ✅ You need multi-cloud portability
- ✅ Complex microservices architecture (10+ services)
- ✅ Enterprise compliance requirements
- ✅ Very high scale (1M+ requests/minute)

**Deployment Plan**:
```bash
# Paris cluster
gcloud container clusters create-auto production-agent \
  --region europe-west1

# Deploy our existing K8s manifests
kubectl apply -f deploy/k8s/

# Add Tokyo cluster
gcloud container clusters create-auto production-agent-asia \
  --region asia-northeast1

# Use global load balancer to route traffic
```

**Cost**: Starts at $620/mo (3 regions)

---

### Not Recommended

❌ **AWS EKS**: Too expensive, too complex for your use case
❌ **Railway/Render**: Not built for global deployment
❌ **Bare Metal**: Too much operational burden
❌ **Cloudflare Workers**: Too limiting for stateful agent

---

## Decision Matrix

Use this to decide:

| Your Priority | Best Platform | Runner-Up |
|---------------|---------------|-----------|
| **Lowest latency globally** | Fly.io | Cloudflare Workers (limited) |
| **Easiest to start** | Fly.io | Railway |
| **Best cost optimization** | Fly.io (small), GKE (large) | Cloud Run |
| **Maximum control** | GKE | AWS EKS |
| **Fastest to deploy** | Fly.io | Render |
| **Best for learning K8s** | GKE | DigitalOcean K8s |
| **Best for Rust** | Fly.io | Any (Docker works everywhere) |
| **Enterprise compliance** | GKE/EKS | Fly.io |

---

## Migration Path

**Start on Fly.io, migrate to GKE if needed**:

**When to migrate from Fly.io to GKE**:
1. ❌ You're spending >$2,000/mo on Fly.io (K8s becomes cheaper)
2. ❌ You need features Fly.io doesn't have (Redpanda cluster, etc.)
3. ❌ Enterprise compliance requires specific cloud provider
4. ❌ You've hired a DevOps team
5. ❌ You need multi-cloud for risk management

**Until then**: Stay on Fly.io! 🚀

**Migration is straightforward**:
- We already have Docker images ✅
- We already have K8s manifests ✅
- Just point DNS to new cluster
- Zero code changes needed

---

## Specific Recommendations for Your Agent

### Phase 1: MVP (Paris Only) - Use Fly.io

```bash
# Total setup time: ~1 hour
fly launch
fly postgres create
fly redis create
fly deploy

# Cost: $50/mo
# Latency in Paris: 10-20ms
```

### Phase 2: European Expansion

```bash
# Add London and Frankfurt
fly regions add lhr fra

# Cost: $100/mo
# Coverage: All of Europe with <30ms
```

### Phase 3: Global Expansion

```bash
# Add Asia and Americas
fly regions add nrt sjc ewr

# Cost: $200/mo
# Coverage: Global with <50ms everywhere
```

### Phase 4: Scale (If You Outgrow Fly.io)

**Option A: Stay on Fly.io, upgrade instances**
- Vertical scaling (bigger machines)
- More replicas per region
- Cost: $500-2,000/mo

**Option B: Migrate to GKE**
- Use existing K8s manifests
- Better for >$2,000/mo scale
- Cost: Starts at $620/mo, scales to $5,000+

---

## Redpanda / Kafka Consideration

**Challenge**: None of the PaaS platforms offer managed Redpanda

**Options**:

1. **Self-host on Fly.io**:
   ```bash
   # Deploy Redpanda as regular app
   fly deploy redpanda-broker-1
   fly deploy redpanda-broker-2
   fly deploy redpanda-broker-3
   ```
   - Works, but you manage it
   - Cost: +$100/mo

2. **Use Upstash Kafka** (Managed Kafka-compatible):
   - Serverless Kafka (Redpanda-compatible protocol)
   - Global replication
   - $10/mo + usage
   - ✅ RECOMMENDED for Fly.io deployment

3. **Use GKE with Redpanda Operator**:
   - Full Redpanda cluster in K8s
   - Better for high throughput
   - Requires K8s

**Recommendation**:
- Start without event bus (not critical for MVP)
- Add Upstash Kafka when you need cross-agent communication
- Or just use Fly.io's PostgreSQL for event sourcing (LISTEN/NOTIFY)

---

## Final Recommendation

**For your AI agent with global deployment requirements**:

### 🏆 Winner: Fly.io

**Reasons**:
1. ✅ Built for your exact use case (global, regional optimization)
2. ✅ Start in Paris, expand incrementally
3. ✅ Best cost ($50 → $200 → $500/mo path)
4. ✅ Excellent DX (developer experience)
5. ✅ No K8s complexity to start
6. ✅ Easy to migrate to K8s later if needed
7. ✅ Our Docker images work as-is

**Action Plan**:
```bash
# This week
1. Sign up for Fly.io
2. Deploy to Paris
3. Test with your Anthropic API key

# Next month
4. Monitor usage and latency
5. Add Tokyo region when you get Asian users

# In 3 months
6. Full global deployment (4 regions)
7. ~$200/mo for global, production-ready agent
```

**Backup Plan**: GKE Autopilot
- If Fly.io doesn't work out
- If you need K8s for other reasons
- If you hire DevOps team

---

## Resources

**Fly.io**:
- Docs: https://fly.io/docs/
- Rust Guide: https://fly.io/docs/languages-and-frameworks/rust/
- Global Deployment: https://fly.io/docs/reference/regions/
- Pricing: https://fly.io/docs/about/pricing/

**GKE**:
- Autopilot: https://cloud.google.com/kubernetes-engine/docs/concepts/autopilot-overview
- Multi-region: https://cloud.google.com/kubernetes-engine/docs/concepts/multi-cluster-ingress
- Pricing: https://cloud.google.com/kubernetes-engine/pricing

**Cost Calculators**:
- Fly.io: https://fly.io/docs/about/pricing/ (simple, predictable)
- GCP: https://cloud.google.com/products/calculator
- AWS: https://calculator.aws/

---

**Last Updated**: 2025-11-11
**Next Review**: After Phase 9 Part 8 (Docker Compose) completion
