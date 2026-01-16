# DEX Aggregator

## Overview
Multi-hop DEX aggregator that routes swap requests across multiple orderbooks to find optimal execution paths using BFS.

## Architecture
- `crates/aggregator` - Core library and CLI binary
- `crates/aggregator-utils` - Shared types (OrderbookState, SwapRequest, Token, etc.)

## Key Patterns
- `scc::HashMap` for lock-free concurrent orderbook state
- Token-bucket rate limiting with atomic operations
- Circuit breaker for graceful degradation under failures
- Arena-based BFS for memory-efficient route finding
- Arc-wrapped snapshots to avoid per-request cloning

## Commands
```bash
cargo test --all              # Run all tests
cargo clippy                  # Lint check
cargo fmt --check             # Format check
cargo llvm-cov --html         # Generate coverage report (requires cargo-llvm-cov)
cargo deny check              # Dependency audit (requires cargo-deny)
```

## Error Handling
- Library code uses `thiserror` for typed errors
- CLI uses `Box<dyn std::error::Error>` at boundaries
- Errors are sanitized before returning to clients (no token existence leakage)
