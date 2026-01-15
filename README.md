# DEX Aggregator

A high-performance decentralized exchange (DEX) aggregator written in Rust. This system finds optimal trading routes across multiple orderbooks using BFS-based pathfinding, with built-in health monitoring, staleness detection, and circuit breaker protection.

## Features

- **Multi-hop routing**: Finds the best path across multiple token pairs (up to 3 hops)
- **Lock-free concurrency**: Uses `scc::HashMap` for high-throughput orderbook updates without reader-writer contention
- **Orderbook health validation**: Detects and rejects crossed spreads and empty books
- **Staleness tracking**: Skips stale orderbooks (>30s without update) during routing
- **Circuit breaker**: Graceful degradation under failure conditions
- **Metrics collection**: Atomic counters for events, requests, failures, and skipped books

## Architecture

The system consists of two main processing pipelines:

1. **Event Processing**: Ingests orderbook updates, validates health, and maintains the token graph
2. **Request Processing**: Handles swap requests, finds optimal routes via BFS, and enforces slippage protection

See [ARCHITECTURE.md](ARCHITECTURE.md) for detailed design decisions, scale analysis, and memory considerations.

## Quick Start

### Prerequisites

- Rust 1.82+ ([installation guide](https://www.rust-lang.org/tools/install))

### Running

```bash
RUST_LOG=aggregator=info cargo run --release -p aggregator -- run
```

### CLI Options

| Flag | Default | Description |
|------|---------|-------------|
| `--num-tokens` | 10 | Number of tokens in the simulation |
| `--num-orderbooks` | 20 | Number of orderbook pairs |
| `--events-per-second` | 100 | Orderbook update rate |
| `--requests-per-second` | 10 | Swap request rate |
| `--slippage-tolerance-bps` | 100 | Slippage tolerance (basis points) |

### Running Tests

```bash
cargo test --all
```

### Test Coverage

```bash
# Install coverage tool
cargo install cargo-llvm-cov

# Generate HTML report
cargo llvm-cov --html

# View report
open target/llvm-cov/html/index.html
```

## Project Structure

```
crates/
├── aggregator/
│   └── src/
│       ├── core/           # Main implementation
│       │   ├── state.rs        # AggregatorState, CircuitBreaker, Metrics
│       │   ├── routing.rs      # BFS route finding with snapshots
│       │   ├── matching.rs     # Order matching logic
│       │   ├── event_processor.rs
│       │   └── request_processor.rs
│       ├── backend/        # Simulation threads
│       └── cli/            # Command-line interface
└── aggregator-utils/       # Shared types and utilities
```

## Key Design Decisions

| Decision | Rationale |
|----------|-----------|
| `scc::HashMap` over `RwLock` | Eliminates reader-writer contention under high write load |
| Point-in-time snapshots | Ensures consistent routing while allowing concurrent updates |
| 3-hop maximum | Bounds search complexity while covering typical DEX routes |
| Circuit breaker | Prevents cascade failures when infrastructure degrades |

## Performance Characteristics

| Metric | Target |
|--------|--------|
| Orderbook updates | 10K/sec |
| Route requests | 1K/sec |
| Concurrent orderbooks | 10K pairs |

## License

MIT
