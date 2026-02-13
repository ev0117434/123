# Binance Futures WebSocket Writer - Промпт для реализации

## Задача

Создать Rust приложение для записи книги ордеров Binance Futures в ClickHouse через WebSocket.

## Требования

### 1. WebSocket подключения
- URL: `wss://fstream.binance.com/stream?streams=<symbol1>@bookTicker/<symbol2>@bookTicker/...`
- Формат стрима: `{symbol}@bookTicker` (lowercase)
- Макс. 200 символов на одно соединение
- Разбить на chunks по 100 символов для надежности
- Задержка 1 сек между запуском соединений

### 2. Обработка данных
Входящий JSON:
```json
{
  "stream": "btcusdt@bookTicker",
  "data": {
    "u": 123456789,
    "s": "BTCUSDT",
    "b": "50000.00",
    "B": "10.5",
    "a": "50001.00",
    "A": "8.3",
    "T": 1234567890000,
    "E": 1234567890500
  }
}
```

Конвертировать в структуру для ClickHouse:
- `update_id` (u) → UInt64
- `symbol` (s) → String
- `best_bid_price` (b) → Decimal64(8)
- `best_bid_qty` (B) → Decimal64(8)
- `best_ask_price` (a) → Decimal64(8)
- `best_ask_qty` (A) → Decimal64(8)
- `transaction_time` (T) → DateTime64(3) миллисекунды
- `event_time` (E) → DateTime64(3) миллисекунды

### 3. Batch запись в ClickHouse
- Батчи по **1000 записей** ИЛИ **каждые 100ms** (что наступит раньше)
- INSERT через `clickhouse-rs` с нативным протоколом
- CREATE TABLE с движком **ReplacingMergeTree**:
```sql
CREATE TABLE IF NOT EXISTS futures_book_ticker (
    update_id UInt64,
    symbol String,
    best_bid_price Decimal64(8),
    best_bid_qty Decimal64(8),
    best_ask_price Decimal64(8),
    best_ask_qty Decimal64(8),
    transaction_time DateTime64(3),
    event_time DateTime64(3)
) ENGINE = ReplacingMergeTree()
ORDER BY (symbol, transaction_time, update_id);
```

### 4. Конфигурация (config.toml)
```toml
[clickhouse]
url = "tcp://localhost:9000"
database = "binance"
table = "futures_book_ticker"
batch_size = 1000
batch_interval_ms = 100

[websocket]
url = "wss://fstream.binance.com/stream"
chunk_size = 100
startup_delay_ms = 1000

[symbols]
symbols = ["BTCUSDT", "ETHUSDT", "BNBUSDT", ...]
```

### 5. Обработка ошибок
- **Переподключение WebSocket** с exponential backoff (1s, 2s, 4s, 8s, макс 30s)
- **Retry ClickHouse** при ошибках записи (3 попытки)
- Graceful shutdown на SIGTERM/SIGINT с flush буфера

### 6. Зависимости
```toml
[dependencies]
tokio = { version = "1", features = ["full"] }
tokio-tungstenite = { version = "0.21", features = ["native-tls"] }
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
clickhouse = "0.11"
toml = "0.8"
futures = "0.3"
```

## Структура проекта

```
src/
├── main.rs              # Entry point, инициализация
├── config.rs            # Загрузка config.toml
├── websocket.rs         # WebSocket клиент, подключение
├── models.rs            # Структуры данных
├── clickhouse.rs        # ClickHouse writer с батчингом
└── utils.rs             # Helpers (lowercase, backoff)
```

## Ключевые моменты

1. **TLS обязательно**: `tokio-tungstenite` с `native-tls` feature
2. **Символы lowercase**: `BTCUSDT` → `btcusdt` для стримов
3. **Время в миллисекундах**: конвертировать в `DateTime64(3)`
4. **Batch оптимизация**: не ждать заполнения батча если прошло 100ms
5. **Chunking символов**: не больше 100 на соединение
6. **Graceful shutdown**: обработать Ctrl+C, записать остаток буфера

## Запуск

```bash
cargo build --release
./target/release/binance-futures-writer
```

---

**Цель**: Высокопроизводительный, надежный writer с минимальной задержкой записи (batch 100ms).
