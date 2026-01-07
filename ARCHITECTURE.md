# DEX Aggregator Architecture

## Overview

This document describes the architectural decisions, scale considerations, and operational characteristics of the DEX aggregator system.

## System Architecture

```
                    ┌─────────────────────────────────────────────────┐
                    │               AggregatorState                   │
                    │  ┌─────────────────────────────────────────┐   │
   OrderbookEvent   │  │  scc::HashMap<(base, quote), Orderbook> │   │
        │           │  └─────────────────────────────────────────┘   │
        ▼           │  ┌─────────────────────────────────────────┐   │
┌───────────────┐   │  │  scc::HashMap<token, Vec<GraphEdge>>    │   │
│ EventProcessor│──▶│  └─────────────────────────────────────────┘   │
└───────────────┘   │  ┌─────────────────────────────────────────┐   │
                    │  │  scc::HashMap<(base, quote), BookMeta>  │   │
                    │  └─────────────────────────────────────────┘   │
                    │  ┌──────────────┐ ┌─────────────────────┐      │
                    │  │   Metrics    │ │   CircuitBreaker    │      │
                    │  └──────────────┘ └─────────────────────┘      │
                    └─────────────────────────────────────────────────┘
                                          │
                                          ▼
                    ┌─────────────────────────────────────────────────┐
    SwapRequest     │              RequestProcessor                   │
        │           │  1. Circuit breaker check                       │
        ▼           │  2. Input validation                            │
┌───────────────┐   │  3. Take routing snapshot                       │
│RequestProcessor──▶│  4. BFS to find best route                      │
└───────────────┘   │  5. Record success/failure                      │
        │           └─────────────────────────────────────────────────┘
        ▼
   SwapResponse
```

## Concurrency Model: scc::HashMap

### Why Not RwLock?

Initial profiling with RwLock<HashMap> showed problematic characteristics:

- **Write latency**: 150-300us under contention (100 orderbook updates/sec)
- **Reader starvation**: Frequent writes caused readers to queue
- **Head-of-line blocking**: A single slow writer blocks all readers

### scc::HashMap Properties

The `scc` crate provides lock-free concurrent hashmaps with:

- **Per-bucket granularity**: Concurrent access to different keys never blocks
- **No reader-writer tradeoff**: Reads proceed without blocking writes
- **Interior mutability**: Methods take `&self`, not `&mut self`

### Memory Trade-offs

scc::HashMap uses more memory per entry than std::HashMap due to:
- Per-bucket metadata for concurrency control
- Epoch-based reclamation bookkeeping

At scale (10K orderbooks), expect ~20-30% memory overhead vs std::HashMap.

## Routing Snapshot Design

### The Problem

BFS route finding visits multiple orderbooks. Without isolation, concurrent updates could cause:
- Inconsistent pricing across hops
- Routing through partially-updated state
- Non-deterministic results

### Solution: Point-in-time Snapshot

```rust
struct RoutingSnapshot {
    orderbooks: HashMap<(Address, Address), OrderbookState>,
    graph_edges: HashMap<Address, Vec<GraphEdge>>,
    meta: HashMap<(Address, Address), BookMeta>,
}
```

Before routing, we clone all relevant state into a local snapshot. This ensures:
- Consistent view across the entire BFS traversal
- No blocking of concurrent writers during routing
- Predictable latency (no lock contention mid-route)

### Memory Analysis at Scale

| Orderbooks | Est. Snapshot Size | Notes |
|------------|-------------------|-------|
| 100 | ~1 MB | Typical test scenario |
| 1,000 | ~10 MB | Small exchange |
| 10,000 | ~100 MB | Large exchange |

Each orderbook averages ~10KB (200 bid levels + 200 ask levels at 24 bytes each).

**Mitigation strategies for extreme scale:**
- Limit snapshot to tokens reachable from input/output (lazy expansion)
- Use reference-counted orderbooks to share immutable state
- Implement incremental snapshots with version vectors

## Health and Staleness Tracking

### BookMeta Per Orderbook

```rust
pub struct BookMeta {
    pub updated_at: Instant,
    pub healthy: bool,
}
```

**Health checks on ingestion:**
- Crossed spread detection (bid >= ask)
- Empty book rejection (no usable levels)

**Staleness during routing:**
- `STALE_TTL = 30 seconds`
- Stale books are skipped during BFS
- Metric tracks skipped books for monitoring

### Rationale

Market data feeds can fail silently. A book that hasn't updated in 30 seconds may have:
- Stale prices leading to failed fills
- Missing liquidity from cancelled orders
- Outdated spread from market moves

Better to route around stale data than serve bad quotes.

## Circuit Breaker

### State Machine

```
     ┌──────────────────────────────────────────┐
     │                                          │
     ▼                                          │
┌─────────┐  5 failures   ┌────────┐  success  ┌───────────┐
│ CLOSED  │──────────────▶│  OPEN  │◀──────────│ HALF-OPEN │
└─────────┘               └────────┘           └───────────┘
     ▲                         │  10s timeout       │
     │                         └───────────────────▶│
     │         success                              │
     └──────────────────────────────────────────────┘
```

### Configuration

```rust
failure_threshold: 5,      // consecutive failures to open
recovery_timeout: 10s,     // time before attempting recovery
```

### What Triggers Failures

Only infrastructure failures affect the circuit breaker:
- Unknown token (no market data received)
- No path found (all books stale/unhealthy)

User errors (same token, zero amount, slippage) do NOT trip the circuit breaker.

### Behavior When Open

Returns `SwapResponse::Failure("service degraded")` immediately without attempting routing. This:
- Prevents cascade failures
- Allows upstream services to failover
- Reduces load during recovery

## Metrics

### Available Counters

| Metric | Type | Description |
|--------|------|-------------|
| `events_total` | Counter | Orderbook updates processed |
| `events_invalid` | Counter | Rejected (crossed/empty) books |
| `requests_total` | Counter | Swap requests received |
| `requests_failed` | Counter | Requests that couldn't be routed |
| `snapshots_taken` | Counter | Routing snapshots created |
| `stale_or_unhealthy_skipped` | Counter | Books skipped during routing |

### Recommended Alerts

```
# High failure rate indicates market data issues
rate(requests_failed) / rate(requests_total) > 0.1

# Stale data indicates feed problems
rate(stale_or_unhealthy_skipped) > 100/s

# Circuit breaker opened
circuit_breaker_state == "open"
```

## Scale Expectations

### Target Throughput

| Component | Target | Bottleneck |
|-----------|--------|------------|
| Event ingestion | 10K updates/sec | Validation + insertion |
| Request processing | 1K routes/sec | BFS + snapshot creation |
| Concurrent orderbooks | 10K pairs | Memory (~100MB state) |

### Latency Targets

| Operation | P50 | P99 |
|-----------|-----|-----|
| Orderbook update | <100us | <1ms |
| Route finding (3 hops) | <1ms | <10ms |
| Snapshot creation | <500us | <2ms |

## Known Limitations

1. **Snapshot memory**: Full state clone on every request. At extreme scale (>50K orderbooks), consider lazy expansion or shared-nothing partitioning.

2. **BFS complexity**: O(V + E) where V = tokens, E = orderbook count. With MAX_HOPS=3, practical complexity is bounded but still grows with graph density.

3. **Single-process**: No distributed state. Horizontal scaling requires partitioning by token or pair prefix.

4. **No persistent storage**: All state is in-memory. Process restart loses all orderbook data.

## Future Improvements

1. **Incremental snapshots**: Version vectors to copy only changed orderbooks since last snapshot.

2. **Dijkstra with pruning**: Replace BFS with priority queue to find optimal route faster.

3. **Distributed state**: Consistent hashing for multi-node deployment with cross-node routing.

4. **Rate limiting**: Per-client request limits to prevent abuse.

5. **Orderbook compression**: Delta encoding for orderbook updates to reduce memory churn.
