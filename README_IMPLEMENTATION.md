# Binance Futures Writer - Implementation

High-performance WebSocket writer for Binance Futures market data to Shared Memory.

## Features

- **Low Latency**: Single-threaded async design with seqlock writes
- **No Float Math**: Decimal price parser for exact precision (1e8 scale)
- **Zero Message Drop**: Burst handling with fairness across connections
- **CPU Pinning**: Affinity to single P-core for consistent performance
- **Chunked Connections**: 512 streams per WebSocket connection
- **Auto Reconnect**: Exponential backoff with graceful recovery
- **Performance Monitoring**: Built-in latency tracking (target <5ms)

## Architecture

### Modules

- `shm.rs` - Shared Memory management with seqlock protocol
- `symbols.rs` - Symbol loading and validation
- `price.rs` - Decimal price parser (no float errors)
- `ws.rs` - WebSocket connection manager with chunking
- `main.rs` - Application orchestration

### Key Design Decisions

1. **Seqlock Protocol**: Lock-free writes for minimal latency
   - Writer: odd seq → write data → even seq (Release fence)
   - Reader: check seq before/after read for consistency

2. **Decimal Price Parsing**: No float arithmetic
   - Parse string as integer arithmetic
   - Round half-up at 9th decimal digit
   - Scale by 1e8 for storage

3. **Single-Threaded Async**: All connections in one event loop
   - CPU affinity to single core
   - No context switching overhead
   - Fairness via message burst limits

4. **Connection Chunking**: 512 streams per WebSocket
   - Binance limit compliance
   - Automatic chunking for any symbol count
   - Independent reconnect per chunk

## Configuration

Constants in code:
- `SUBSCRIBE_FILE`: `/root/siro/dictionaries/subscribe/binance/binance_futures.txt`
- `SYMBOLS_TSV`: `/root/siro/dictionaries/configs/symbols.tsv`
- `SHM_PATH`: `/dev/shm/quotes_v1.dat`
- `SOURCE_ID`: `1`
- `CHUNK_SIZE`: `512` streams per connection

Environment variables:
- `CPU_CORE`: CPU core for affinity (default: 0)

## File Formats

### subscribe/binance_futures.txt
```
BTCUSDT
ETHUSDT
...
```

### symbols.tsv
```
<symbol_id>\t<SYMBOL>
1	BTCUSDT
2	ETHUSDT
```

## SHM Format

### Header (4096 bytes)
- Magic: `QSHM1\0\0\0`
- Record size: 64 bytes
- Price scale: 1e8
- Timestamp scale: 1e6 (microseconds!)

### Record (64 bytes)
```rust
struct Quote64 {
    seq: AtomicU64,     // seqlock counter
    source_id: u64,     // SOURCE_ID (1)
    symbol_id: u64,     // from symbols.tsv
    bid: i64,           // bid_price * 1e8
    ask: i64,           // ask_price * 1e8
    ts: i64,            // monotonic_us
    reserved0: u64,
    reserved1: u64,
}
```

### Slot Indexing
```
idx = source_id * n_symbols + symbol_id
offset = 4096 + idx * 64
```

## Error Codes

- `exit(1)` - SHM validation failed
- `exit(2)` - WebSocket connection failed (fatal)
- `exit(10)` - Unknown symbol received
- `exit(11)` - Invalid slot access
- `exit(20)` - Symbol validation failed

## Performance

Target: <5ms processing time per message

Monitoring:
- `max_proc_us` - Maximum processing time
- `over_5000us_count` - Messages exceeding 5ms
- `total_messages` - Total processed

Stats printed on Ctrl+C.

## Building

```bash
cargo build --release
```

## Running

```bash
# Default (CPU core 0)
cargo run --release

# Specific CPU core
CPU_CORE=4 cargo run --release
```

## Testing

```bash
# All tests
cargo test

# Price parser tests
cargo test parse_price

# SHM tests
cargo test shm
```

## Dependencies

- `tokio` - Async runtime
- `tokio-tungstenite` - WebSocket client
- `serde_json` - JSON parsing
- `memmap2` - Memory mapping
- `libc` - System calls (CPU affinity, clock_gettime)
- `anyhow` - Error handling

## Safety Notes

1. **No logs on hot path** - Only error logging
2. **No allocations per message** - Pre-allocated structures
3. **No float arithmetic** - Decimal parsing only
4. **Atomic operations** - Proper memory ordering
5. **Single writer per slot** - No write conflicts

## Future Improvements

- [ ] Histogram statistics (latency buckets)
- [ ] Separate SHM area for statistics
- [ ] Rolling reconnect (24h) for connection refresh
- [ ] Message rate limiting
- [ ] Health check endpoint
