use std::fs::OpenOptions;
use std::sync::atomic::{AtomicU64, Ordering};
use anyhow::{bail, Context, Result};
use memmap2::MmapMut;

// Constants from spec
const MAGIC: &[u8; 8] = b"QSHM1\0\0\0";
const EXPECTED_HEADER_SIZE: u64 = 4096;
const EXPECTED_RECORD_SIZE: u64 = 64;
const EXPECTED_RECORDS_OFFSET: u64 = 4096;
const EXPECTED_PRICE_SCALE: u64 = 100_000_000; // 1e8
const EXPECTED_TS_SCALE: u64 = 1_000_000; // 1e6 (microseconds!)

/// SHM Header (first 4096 bytes)
#[repr(C)]
#[derive(Debug)]
pub struct ShmHeader {
    pub magic: [u8; 8],
    pub version: u64,
    pub header_size: u64,
    pub record_size: u64,
    pub records_offset: u64,
    pub price_scale: u64,
    pub ts_scale: u64,
    pub n_sources: u64,
    pub n_symbols: u64,
    pub n_records: u64,
    pub shm_total_size: u64,
}

/// Quote record (64 bytes, cache-line aligned)
#[repr(C, align(64))]
pub struct Quote64 {
    pub seq: AtomicU64,
    pub source_id: u64,
    pub symbol_id: u64,
    pub bid: i64,
    pub ask: i64,
    pub ts: i64,
    pub reserved0: u64,
    pub reserved1: u64,
}

const _: () = assert!(std::mem::size_of::<Quote64>() == 64);

impl Quote64 {
    /// Initialize slot with constant fields (source_id, symbol_id)
    /// This is done once at startup for each slot
    pub fn init_slot(&mut self, source_id: u64, symbol_id: u64) {
        self.seq.store(0, Ordering::Relaxed);
        self.source_id = source_id;
        self.symbol_id = symbol_id;
        self.bid = 0;
        self.ask = 0;
        self.ts = 0;
        self.reserved0 = 0;
        self.reserved1 = 0;
    }

    /// Write quote using seqlock protocol
    /// CRITICAL: This must be lock-free and minimal latency
    #[inline(always)]
    pub fn write(&self, bid: i64, ask: i64, ts: i64) {
        // Load current seq (should be even)
        let seq0 = self.seq.load(Ordering::Relaxed);

        // Mark as "writing" (odd)
        self.seq.store(seq0.wrapping_add(1), Ordering::Release);

        // Write data fields
        // SAFETY: We have exclusive access to this slot (one writer per slot)
        unsafe {
            let ptr = self as *const Quote64 as *mut Quote64;
            (*ptr).bid = bid;
            (*ptr).ask = ask;
            (*ptr).ts = ts;
        }

        // Mark as "complete" (even), with Release fence
        self.seq.store(seq0.wrapping_add(2), Ordering::Release);
    }

    /// Read quote using seqlock protocol (for testing/debugging)
    #[allow(dead_code)]
    pub fn read(&self) -> Option<(u64, u64, i64, i64, i64)> {
        for _ in 0..1000 {
            let s1 = self.seq.load(Ordering::Acquire);

            // If odd, writer is in progress
            if (s1 & 1) == 1 {
                continue;
            }

            let sid = self.source_id;
            let sym = self.symbol_id;
            let bid = self.bid;
            let ask = self.ask;
            let ts = self.ts;

            let s2 = self.seq.load(Ordering::Acquire);

            // Check if seq changed during read
            if s1 != s2 {
                continue;
            }

            return Some((sid, sym, bid, ask, ts));
        }
        None
    }
}

/// SHM manager
pub struct ShmManager {
    #[allow(dead_code)]
    mmap: MmapMut,
    records_base: *mut Quote64,
    n_symbols: u64,
    n_sources: u64,
}

unsafe impl Send for ShmManager {}
unsafe impl Sync for ShmManager {}

impl ShmManager {
    /// Open and validate SHM file
    pub fn open(path: &str) -> Result<Self> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .with_context(|| format!("Failed to open SHM file: {}", path))?;

        let metadata = file.metadata()
            .context("Failed to get file metadata")?;
        let file_size = metadata.len();

        let mut mmap = unsafe {
            MmapMut::map_mut(&file)
                .context("Failed to mmap file")?
        };

        // Parse and validate header
        let header = unsafe {
            &*(mmap.as_ptr() as *const ShmHeader)
        };

        // Validate magic
        if &header.magic != MAGIC {
            bail!("Invalid magic: expected {:?}, got {:?}", MAGIC, header.magic);
        }

        // Validate header_size
        if header.header_size != EXPECTED_HEADER_SIZE {
            bail!("Invalid header_size: expected {}, got {}", EXPECTED_HEADER_SIZE, header.header_size);
        }

        // Validate record_size
        if header.record_size != EXPECTED_RECORD_SIZE {
            bail!("Invalid record_size: expected {}, got {}", EXPECTED_RECORD_SIZE, header.record_size);
        }

        // Validate records_offset
        if header.records_offset != EXPECTED_RECORDS_OFFSET {
            bail!("Invalid records_offset: expected {}, got {}", EXPECTED_RECORDS_OFFSET, header.records_offset);
        }

        // Validate price_scale
        if header.price_scale != EXPECTED_PRICE_SCALE {
            bail!("Invalid price_scale: expected {}, got {}", EXPECTED_PRICE_SCALE, header.price_scale);
        }

        // Validate ts_scale (CRITICAL: must be 1e6 for microseconds)
        if header.ts_scale != EXPECTED_TS_SCALE {
            bail!("Invalid ts_scale: expected {} (1e6), got {}", EXPECTED_TS_SCALE, header.ts_scale);
        }

        // Validate total size
        if header.shm_total_size != file_size {
            bail!("Size mismatch: header says {}, file is {}", header.shm_total_size, file_size);
        }

        // Validate n_records
        let expected_records = header.n_sources * header.n_symbols;
        if header.n_records != expected_records {
            bail!("Invalid n_records: expected {}, got {}", expected_records, header.n_records);
        }

        // Calculate records base pointer
        let records_base = unsafe {
            mmap.as_mut_ptr().add(header.records_offset as usize) as *mut Quote64
        };

        eprintln!("[SHM] Opened: {} sources, {} symbols, {} records",
                  header.n_sources, header.n_symbols, header.n_records);

        Ok(Self {
            mmap,
            records_base,
            n_symbols: header.n_symbols,
            n_sources: header.n_sources,
        })
    }

    /// Get slot for (source_id, symbol_id)
    #[inline(always)]
    pub fn get_slot(&self, source_id: u64, symbol_id: u64) -> Result<&Quote64> {
        if source_id >= self.n_sources {
            bail!("source_id {} out of range (max {})", source_id, self.n_sources);
        }
        if symbol_id >= self.n_symbols {
            bail!("symbol_id {} out of range (max {})", symbol_id, self.n_symbols);
        }

        let idx = source_id * self.n_symbols + symbol_id;

        unsafe {
            let ptr = self.records_base.add(idx as usize);
            Ok(&*ptr)
        }
    }

    /// Initialize slot with constant fields
    pub fn init_slot(&mut self, source_id: u64, symbol_id: u64) -> Result<()> {
        if source_id >= self.n_sources {
            bail!("source_id {} out of range", source_id);
        }
        if symbol_id >= self.n_symbols {
            bail!("symbol_id {} out of range", symbol_id);
        }

        let idx = source_id * self.n_symbols + symbol_id;

        unsafe {
            let ptr = self.records_base.add(idx as usize);
            (*ptr).init_slot(source_id, symbol_id);
        }

        Ok(())
    }
}

/// Get monotonic timestamp in microseconds
#[inline(always)]
pub fn monotonic_us() -> i64 {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    unsafe {
        libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts);
    }
    ts.tv_sec * 1_000_000 + ts.tv_nsec / 1_000
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_quote64_size() {
        assert_eq!(std::mem::size_of::<Quote64>(), 64);
    }

    #[test]
    fn test_seqlock() {
        let quote = Quote64 {
            seq: AtomicU64::new(0),
            source_id: 1,
            symbol_id: 10,
            bid: 0,
            ask: 0,
            ts: 0,
            reserved0: 0,
            reserved1: 0,
        };

        // Write
        quote.write(10000000000, 10000100000, 123456789);

        // Read
        let result = quote.read();
        assert!(result.is_some());
        let (sid, sym, bid, ask, ts) = result.unwrap();
        assert_eq!(sid, 1);
        assert_eq!(sym, 10);
        assert_eq!(bid, 10000000000);
        assert_eq!(ask, 10000100000);
        assert_eq!(ts, 123456789);
    }
}
