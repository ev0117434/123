mod shm;
mod symbols;
mod price;
mod ws;

use anyhow::{Context, Result};
use std::collections::HashMap;
use std::process;
use std::sync::Arc;

// Constants from spec
const SUBSCRIBE_FILE: &str = "/root/siro/dictionaries/subscribe/binance/binance_futures.txt";
const SYMBOLS_TSV: &str = "/root/siro/dictionaries/configs/symbols.tsv";
const SHM_PATH: &str = "/dev/shm/quotes_v1.dat";
const SOURCE_ID: u64 = 1;

/// Main application state
struct App {
    shm: Arc<shm::ShmManager>,
    symbol_id_map: Arc<HashMap<String, u64>>,
    perf_stats: Arc<ws::PerfStats>,
}

impl App {
    /// Initialize application
    fn new() -> Result<Self> {
        eprintln!("[INIT] Loading symbols...");

        // Load symbols.tsv
        let symbol_map = symbols::load_symbols_tsv(SYMBOLS_TSV)
            .context("Failed to load symbols.tsv")?;

        // Load subscribe list
        let subscribe_list = symbols::load_subscribe_list(SUBSCRIBE_FILE)
            .context("Failed to load subscribe list")?;

        // Validate all symbols exist
        symbols::validate_symbols(&subscribe_list, &symbol_map)
            .context("Symbol validation failed")?;

        eprintln!("[INIT] All {} symbols validated", subscribe_list.len());

        // Create symbol_id lookup map
        let symbol_id_map = symbols::create_symbol_id_map(&subscribe_list, &symbol_map)
            .context("Failed to create symbol_id map")?;

        // Open and validate SHM
        eprintln!("[INIT] Opening SHM: {}", SHM_PATH);
        let mut shm = shm::ShmManager::open(SHM_PATH)
            .context("Failed to open SHM")?;

        // Initialize slots for all subscribed symbols
        eprintln!("[INIT] Initializing SHM slots...");
        for (symbol, &symbol_id) in &symbol_id_map {
            shm.init_slot(SOURCE_ID, symbol_id)
                .with_context(|| format!("Failed to init slot for {}", symbol))?;
        }

        eprintln!("[INIT] Initialization complete!");

        Ok(Self {
            shm: Arc::new(shm),
            symbol_id_map: Arc::new(symbol_id_map),
            perf_stats: Arc::new(ws::PerfStats::new()),
        })
    }

    /// Create message handler
    fn create_handler(&self) -> Arc<dyn Fn(ws::BookTickerData) + Send + Sync> {
        let shm = self.shm.clone();
        let symbol_id_map = self.symbol_id_map.clone();
        let perf_stats = self.perf_stats.clone();

        Arc::new(move |data: ws::BookTickerData| {
            let t_start = shm::monotonic_us();

            // Look up symbol_id
            let symbol_id = match symbol_id_map.get(&data.symbol) {
                Some(&id) => id,
                None => {
                    eprintln!("[ERROR] Unknown symbol: {}", data.symbol);
                    process::exit(10);
                }
            };

            // Parse prices (no float!)
            let bid = match price::parse_price_i64_1e8(&data.bid_price) {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("[ERROR] Failed to parse bid price '{}': {}", data.bid_price, e);
                    return;
                }
            };

            let ask = match price::parse_price_i64_1e8(&data.ask_price) {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("[ERROR] Failed to parse ask price '{}': {}", data.ask_price, e);
                    return;
                }
            };

            // Get timestamp (monotonic microseconds)
            let ts = shm::monotonic_us();

            // Get slot and write
            let slot = match shm.get_slot(SOURCE_ID, symbol_id) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("[ERROR] Failed to get slot for symbol_id {}: {}", symbol_id, e);
                    process::exit(11);
                }
            };

            // Write to SHM using seqlock
            slot.write(bid, ask, ts);

            // Record performance
            let t_end = shm::monotonic_us();
            let proc_us = (t_end - t_start) as u64;
            perf_stats.record(proc_us);

            // Optional: log slow messages (but not on hot path in production!)
            if proc_us > 5000 {
                eprintln!("[WARN] Slow message processing: {} µs for {}", proc_us, data.symbol);
            }
        })
    }

    /// Run the application
    async fn run(&self, subscribe_list: Vec<String>) -> Result<()> {
        // Set up signal handler for graceful shutdown
        let perf_stats = self.perf_stats.clone();
        tokio::spawn(async move {
            tokio::signal::ctrl_c().await.ok();
            eprintln!("\n[SHUTDOWN] Received Ctrl+C, printing stats...");
            perf_stats.report();
            process::exit(0);
        });

        // Create message handler
        let handler = self.create_handler();

        // Create WebSocket manager
        let ws_manager = ws::WsManager::new(subscribe_list, handler);

        // Run all connections
        eprintln!("[MAIN] Starting WebSocket connections...");
        ws_manager.run_all().await?;

        Ok(())
    }
}

/// Set CPU affinity to single core
fn set_cpu_affinity(cpu: usize) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        use libc::{cpu_set_t, sched_setaffinity, CPU_SET, CPU_ZERO};
        use std::mem;

        unsafe {
            let mut cpu_set: cpu_set_t = mem::zeroed();
            CPU_ZERO(&mut cpu_set);
            CPU_SET(cpu, &mut cpu_set);

            let result = sched_setaffinity(
                0, // current thread
                mem::size_of::<cpu_set_t>(),
                &cpu_set,
            );

            if result != 0 {
                anyhow::bail!("Failed to set CPU affinity: {}", std::io::Error::last_os_error());
            }
        }

        eprintln!("[CPU] Affinity set to core {}", cpu);
        Ok(())
    }

    #[cfg(not(target_os = "linux"))]
    {
        eprintln!("[CPU] CPU affinity not supported on this platform");
        Ok(())
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    eprintln!("=== Binance Futures Writer ===");
    eprintln!("Version: 0.1.0");
    eprintln!("Source ID: {}", SOURCE_ID);
    eprintln!();

    // Set CPU affinity to core 0 (or use env var)
    let cpu = std::env::var("CPU_CORE")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    if let Err(e) = set_cpu_affinity(cpu) {
        eprintln!("[WARN] Failed to set CPU affinity: {}", e);
    }

    // Initialize application
    let app = match App::new() {
        Ok(app) => app,
        Err(e) => {
            eprintln!("[FATAL] Initialization failed: {:?}", e);
            process::exit(1);
        }
    };

    // Load subscribe list again for WS connections
    let subscribe_list = symbols::load_subscribe_list(SUBSCRIBE_FILE)
        .context("Failed to load subscribe list")?;

    // Run application
    if let Err(e) = app.run(subscribe_list).await {
        eprintln!("[FATAL] Application error: {:?}", e);
        process::exit(2);
    }

    Ok(())
}
