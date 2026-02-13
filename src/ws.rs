use anyhow::{Context, Result};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio_tungstenite::{connect_async, tungstenite::Message};
use std::sync::Arc;

const WS_BASE: &str = "wss://fstream.binance.com";
const CHUNK_SIZE: usize = 100; // Max streams per connection

/// Binance Futures bookTicker message
#[derive(Debug, Deserialize, Serialize)]
pub struct BookTickerData {
    #[serde(rename = "s")]
    pub symbol: String,
    #[serde(rename = "b")]
    pub bid_price: String,
    #[serde(rename = "a")]
    pub ask_price: String,
    // We ignore other fields (u, B, A, etc.) for performance
}

/// Wrapper message from combined stream
#[derive(Debug, Deserialize)]
pub struct StreamMessage {
    #[allow(dead_code)]
    pub stream: String,
    pub data: BookTickerData,
}

/// Create WebSocket URL for a chunk of symbols
fn create_ws_url(symbols: &[String]) -> String {
    let streams: Vec<String> = symbols
        .iter()
        .map(|s| format!("{}@bookTicker", s.to_lowercase()))
        .collect();

    format!("{}/stream?streams={}", WS_BASE, streams.join("/"))
}

/// Split symbols into chunks of CHUNK_SIZE
pub fn chunk_symbols(symbols: &[String]) -> Vec<Vec<String>> {
    symbols
        .chunks(CHUNK_SIZE)
        .map(|chunk| chunk.to_vec())
        .collect()
}

/// Message handler callback
pub type MessageHandler = Arc<dyn Fn(BookTickerData) + Send + Sync>;

/// WebSocket connection manager
pub struct WsConnection {
    symbols: Vec<String>,
    handler: MessageHandler,
}

impl WsConnection {
    pub fn new(symbols: Vec<String>, handler: MessageHandler) -> Self {
        Self { symbols, handler }
    }

    /// Connect and start receiving messages
    /// Returns when connection closes or error occurs
    pub async fn run(&self) -> Result<()> {
        let url = create_ws_url(&self.symbols);

        eprintln!("[WS] Connecting to {} streams...", self.symbols.len());

        let (ws_stream, _) = connect_async(&url)
            .await
            .with_context(|| format!("Failed to connect to {}", url))?;

        eprintln!("[WS] Connected! Receiving messages...");

        let (mut write, mut read) = ws_stream.split();

        // Spawn ping task
        let ping_task = tokio::spawn(async move {
            loop {
                tokio::time::sleep(tokio::time::Duration::from_secs(30)).await;
                if write.send(Message::Ping(vec![])).await.is_err() {
                    break;
                }
            }
        });

        // Process messages
        while let Some(msg) = read.next().await {
            match msg {
                Ok(Message::Text(text)) => {
                    // Parse and handle message
                    match serde_json::from_str::<StreamMessage>(&text) {
                        Ok(stream_msg) => {
                            (self.handler)(stream_msg.data);
                        }
                        Err(e) => {
                            eprintln!("[WS] Failed to parse message: {}", e);
                            // Don't exit on parse errors - might be other message types
                        }
                    }
                }
                Ok(Message::Ping(data)) => {
                    // Tungstenite handles pong automatically
                    drop(data);
                }
                Ok(Message::Pong(_)) => {
                    // Expected response to our pings
                }
                Ok(Message::Close(_)) => {
                    eprintln!("[WS] Connection closed by server");
                    break;
                }
                Err(e) => {
                    eprintln!("[WS] Error receiving message: {}", e);
                    break;
                }
                _ => {}
            }
        }

        ping_task.abort();

        Ok(())
    }
}

/// Backoff calculator for reconnections
struct BackoffCalculator {
    attempt: u32,
    delays_ms: Vec<u64>,
    max_delay_ms: u64,
}

impl BackoffCalculator {
    fn new() -> Self {
        Self {
            attempt: 0,
            delays_ms: vec![200, 500, 1000, 2000, 5000, 10000, 30000],
            max_delay_ms: 30000,
        }
    }

    fn next_delay(&mut self) -> tokio::time::Duration {
        let delay_ms = if (self.attempt as usize) < self.delays_ms.len() {
            self.delays_ms[self.attempt as usize]
        } else {
            self.max_delay_ms
        };

        self.attempt += 1;
        tokio::time::Duration::from_millis(delay_ms)
    }

    fn reset(&mut self) {
        self.attempt = 0;
    }
}

/// Multi-connection manager with fairness
pub struct WsManager {
    connections: Vec<WsConnection>,
}

impl WsManager {
    pub fn new(symbols: Vec<String>, handler: MessageHandler) -> Self {
        let chunks = chunk_symbols(&symbols);
        let n_connections = chunks.len();

        eprintln!("[WS] Creating {} connections for {} symbols", n_connections, symbols.len());

        let connections: Vec<_> = chunks
            .into_iter()
            .map(|chunk| WsConnection::new(chunk, handler.clone()))
            .collect();

        Self { connections }
    }

    /// Run all connections concurrently with exponential backoff
    pub async fn run_all(&self) -> Result<()> {
        // Clone connections for 'static lifetime
        let connections: Vec<WsConnection> = self.connections
            .iter()
            .map(|c| WsConnection {
                symbols: c.symbols.clone(),
                handler: c.handler.clone(),
            })
            .collect();

        let tasks: Vec<_> = connections
            .into_iter()
            .enumerate()
            .map(|(i, conn)| {
                tokio::spawn(async move {
                    // Staggered startup: 200ms delay between connections to avoid rate limits
                    let startup_delay = tokio::time::Duration::from_millis(i as u64 * 200);
                    if startup_delay.as_millis() > 0 {
                        eprintln!("[WS-{}] Waiting {:?} before startup (rate limiting)...", i, startup_delay);
                        tokio::time::sleep(startup_delay).await;
                    }

                    let mut backoff = BackoffCalculator::new();
                    let mut consecutive_errors = 0;

                    loop {
                        eprintln!("[WS-{}] Starting connection (attempt {})...", i, backoff.attempt + 1);

                        match conn.run().await {
                            Ok(_) => {
                                eprintln!("[WS-{}] Connection closed gracefully", i);
                                backoff.reset();
                                consecutive_errors = 0;
                            }
                            Err(e) => {
                                consecutive_errors += 1;
                                eprintln!("[WS-{}] Connection error ({}): {}", i, consecutive_errors, e);

                                // Fatal after too many consecutive errors
                                if consecutive_errors > 10 {
                                    eprintln!("[WS-{}] FATAL: Too many consecutive errors, giving up", i);
                                    std::process::exit(3);
                                }
                            }
                        }

                        // Reconnect with backoff + jitter to avoid thundering herd
                        let base_delay = backoff.next_delay();
                        let jitter_ms = (i as u64 * 50) % 500; // 0-500ms jitter based on connection id
                        let delay = base_delay + tokio::time::Duration::from_millis(jitter_ms);
                        eprintln!("[WS-{}] Reconnecting in {:?}...", i, delay);
                        tokio::time::sleep(delay).await;
                    }
                })
            })
            .collect();

        // Wait for all tasks (they should never complete normally)
        for task in tasks {
            let _ = task.await;
        }

        Ok(())
    }
}

/// Performance statistics
pub struct PerfStats {
    pub max_proc_us: std::sync::atomic::AtomicU64,
    pub over_5000us_count: std::sync::atomic::AtomicU64,
    pub total_messages: std::sync::atomic::AtomicU64,
}

impl PerfStats {
    pub fn new() -> Self {
        Self {
            max_proc_us: std::sync::atomic::AtomicU64::new(0),
            over_5000us_count: std::sync::atomic::AtomicU64::new(0),
            total_messages: std::sync::atomic::AtomicU64::new(0),
        }
    }

    #[inline(always)]
    pub fn record(&self, proc_us: u64) {
        use std::sync::atomic::Ordering;

        self.total_messages.fetch_add(1, Ordering::Relaxed);

        // Update max
        let mut current_max = self.max_proc_us.load(Ordering::Relaxed);
        while proc_us > current_max {
            match self.max_proc_us.compare_exchange_weak(
                current_max,
                proc_us,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(x) => current_max = x,
            }
        }

        // Count > 5000us
        if proc_us > 5000 {
            self.over_5000us_count.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn report(&self) {
        use std::sync::atomic::Ordering;

        let total = self.total_messages.load(Ordering::Relaxed);
        let max = self.max_proc_us.load(Ordering::Relaxed);
        let over5ms = self.over_5000us_count.load(Ordering::Relaxed);

        eprintln!("\n[STATS] Total messages: {}", total);
        eprintln!("[STATS] Max processing time: {} µs", max);
        eprintln!("[STATS] Messages > 5000µs: {}", over5ms);
        if total > 0 {
            eprintln!("[STATS] > 5ms rate: {:.2}%", (over5ms as f64 / total as f64) * 100.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chunk_symbols() {
        let symbols: Vec<String> = (0..1000).map(|i| format!("SYM{}", i)).collect();
        let chunks = chunk_symbols(&symbols);

        // 1000 symbols should make 10 chunks (100 * 10)
        assert_eq!(chunks.len(), 10);
        assert_eq!(chunks[0].len(), 100);
        assert_eq!(chunks[9].len(), 100);
    }

    #[test]
    fn test_create_ws_url() {
        let symbols = vec!["BTCUSDT".to_string(), "ETHUSDT".to_string()];
        let url = create_ws_url(&symbols);

        assert!(url.contains("wss://fstream.binance.com/stream?streams="));
        assert!(url.contains("btcusdt@bookTicker"));
        assert!(url.contains("ethusdt@bookTicker"));
    }
}
