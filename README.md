# Binance Futures Writer

High-performance Rust writer for Binance USDⓈ-M Futures market data to Shared Memory.

## Overview

This writer connects to Binance Futures WebSocket market streams and writes **best bid/best ask** quotes to a shared memory table with minimal latency (<5ms target). It implements a lock-free seqlock protocol for zero-copy reads.

## Features

- ✅ **Ultra-low latency**: Single-threaded async design with seqlock writes
- ✅ **Precision**: Decimal price parser without float errors (1e8 scale)
- ✅ **Zero message drop**: Guaranteed message processing
- ✅ **CPU pinning**: Affinity to single P-core for consistent performance
- ✅ **Auto-scaling**: Handles 512 streams per WebSocket connection
- ✅ **Auto reconnect**: Exponential backoff with graceful recovery
- ✅ **Performance monitoring**: Built-in latency tracking

## Quick Start

```bash
# Build
cargo build --release

# Run (default: CPU core 0)
cargo run --release

# Run on specific CPU core
CPU_CORE=4 cargo run --release
```

## Configuration Files

### `/root/siro/dictionaries/subscribe/binance/binance_futures.txt`
List of symbols to subscribe (one per line):
```
BTCUSDT
ETHUSDT
BNBUSDT
```

### `/root/siro/dictionaries/configs/symbols.tsv`
Symbol ID mapping (tab-separated):
```
1	BTCUSDT
2	ETHUSDT
3	BNBUSDT
```

### Shared Memory
- Path: `/dev/shm/quotes_v1.dat`
- Must be pre-created with correct header
- Source ID: `1` (constant)

## Architecture

### Module Structure

```
src/
├── main.rs      - Application orchestration
├── lib.rs       - Library interface
├── shm.rs       - Shared Memory with seqlock
├── symbols.rs   - Symbol loading/validation
├── price.rs     - Decimal price parser
└── ws.rs        - WebSocket connection manager
```

### Data Flow

```
WebSocket → Parse JSON → Decimal Parser → SHM Write (seqlock)
                ↓                            ↓
        Validate Symbol              Performance Stats
```

### Seqlock Protocol

Writer sequence:
1. Load current `seq` (even)
2. Store `seq + 1` (odd) with Release
3. Write bid, ask, timestamp
4. Store `seq + 2` (even) with Release

Reader sequence:
1. Load `seq` (retry if odd)
2. Read all fields
3. Load `seq` again
4. Retry if seq changed

## Performance

### Target
- Processing time: **< 5ms** per message
- Zero message drops
- Single P-core execution

### Monitoring
- `max_proc_us` - Maximum processing time
- `over_5000us_count` - Messages exceeding 5ms
- `total_messages` - Total processed

Press Ctrl+C to see statistics.

## Testing

```bash
# Run all tests
cargo test

# Run specific module tests
cargo test price     # Decimal parser tests
cargo test shm       # Seqlock tests
cargo test ws        # WebSocket tests

# Run with output
cargo test -- --nocapture
```

### Test Coverage
- ✅ Price parser (12 tests): integers, decimals, rounding, edge cases
- ✅ Seqlock protocol: atomic operations, consistency
- ✅ Symbol management: loading, validation
- ✅ WebSocket chunking: 512 streams per connection

## Error Codes

| Code | Meaning |
|------|---------|
| 1 | SHM validation failed (magic, sizes, scales) |
| 2 | WebSocket connection failed |
| 3 | Too many consecutive connection errors |
| 10 | Unknown symbol received from WebSocket |
| 11 | Invalid slot access (symbol_id out of range) |
| 20 | Symbol validation failed (subscribe list mismatch) |

## Technical Details

### Shared Memory Format

**Header** (4096 bytes):
- Magic: `QSHM1\0\0\0`
- Record size: 64 bytes
- Price scale: 1e8
- Timestamp scale: 1e6 (microseconds)

**Record** (64 bytes):
```rust
struct Quote64 {
    seq: AtomicU64,     // seqlock counter
    source_id: u64,     // 1 (constant)
    symbol_id: u64,     // from symbols.tsv
    bid: i64,           // bid_price * 1e8
    ask: i64,           // ask_price * 1e8
    ts: i64,            // monotonic_us
    reserved0: u64,
    reserved1: u64,
}
```

**Slot indexing**:
```
idx = source_id * n_symbols + symbol_id
offset = 4096 + idx * 64
```

### WebSocket Endpoints

- Base: `wss://fstream.binance.com`
- Stream format: `<symbol_lower>@bookTicker`
- Combined URL: `/stream?streams=btcusdt@bookTicker/ethusdt@bookTicker/...`
- Max per connection: 512 streams

### Reconnect Strategy

Exponential backoff delays:
```
200ms → 500ms → 1s → 2s → 5s → 10s → 30s (max)
```

Fatal after 10 consecutive errors.

## Dependencies

- `tokio` - Async runtime (current_thread flavor)
- `tokio-tungstenite` - WebSocket client
- `serde_json` - JSON parsing
- `memmap2` - Memory mapping
- `libc` - System calls (CPU affinity, monotonic time)
- `anyhow` - Error handling

## Implementation Notes

### Hot Path Optimizations

1. **No allocations** - Pre-allocated structures
2. **No float math** - Integer arithmetic only
3. **No logs** - Only error messages
4. **Atomic operations** - Proper memory ordering
5. **Single writer** - No lock contention

### Safety Guarantees

- One writer per slot (enforced by design)
- Atomic seq operations with Release/Acquire
- Monotonic timestamps (never go backwards)
- Symbol validation before processing

## Documentation

See [README_IMPLEMENTATION.md](README_IMPLEMENTATION.md) for detailed implementation notes.

## License

Internal project - All rights reserved.
