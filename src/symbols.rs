use std::collections::HashMap;
use std::fs;
use anyhow::{bail, Context, Result};

/// Symbol mapping: symbol name -> symbol_id
pub type SymbolMap = HashMap<String, u64>;

/// Load symbols.tsv file
/// Format: <symbol_id>\t<SYMBOL>
pub fn load_symbols_tsv(path: &str) -> Result<SymbolMap> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("Failed to read symbols file: {}", path))?;

    let mut map = HashMap::new();

    for (line_num, line) in content.lines().enumerate() {
        let line = line.trim();

        // Skip empty lines
        if line.is_empty() {
            continue;
        }

        let parts: Vec<&str> = line.split('\t').collect();
        if parts.len() != 2 {
            bail!("Invalid format at line {}: expected <id>\\t<symbol>, got: {}", line_num + 1, line);
        }

        let symbol_id: u64 = parts[0].parse()
            .with_context(|| format!("Invalid symbol_id at line {}: {}", line_num + 1, parts[0]))?;
        let symbol = parts[1].to_uppercase();

        if map.insert(symbol.clone(), symbol_id).is_some() {
            bail!("Duplicate symbol: {}", symbol);
        }
    }

    eprintln!("[SYMBOLS] Loaded {} symbols from {}", map.len(), path);
    Ok(map)
}

/// Load subscribe list file
/// Format: one symbol per line
pub fn load_subscribe_list(path: &str) -> Result<Vec<String>> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("Failed to read subscribe file: {}", path))?;

    let symbols: Vec<String> = content
        .lines()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty())
        .map(|line| line.to_uppercase())
        .collect();

    if symbols.is_empty() {
        bail!("Subscribe list is empty: {}", path);
    }

    eprintln!("[SUBSCRIBE] Loaded {} symbols from {}", symbols.len(), path);
    Ok(symbols)
}

/// Validate that all subscribe symbols exist in symbol map
pub fn validate_symbols(subscribe_list: &[String], symbol_map: &SymbolMap) -> Result<()> {
    for symbol in subscribe_list {
        if !symbol_map.contains_key(symbol) {
            bail!("Symbol {} from subscribe list not found in symbols.tsv", symbol);
        }
    }
    Ok(())
}

/// Create symbol_id lookup map from subscribe list
pub fn create_symbol_id_map(subscribe_list: &[String], symbol_map: &SymbolMap) -> Result<HashMap<String, u64>> {
    let mut result = HashMap::new();

    for symbol in subscribe_list {
        let symbol_id = symbol_map.get(symbol)
            .ok_or_else(|| anyhow::anyhow!("Symbol {} not found in symbols.tsv", symbol))?;
        result.insert(symbol.clone(), *symbol_id);
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_symbol_map() {
        let mut map = SymbolMap::new();
        map.insert("BTCUSDT".to_string(), 1);
        map.insert("ETHUSDT".to_string(), 2);

        assert_eq!(map.get("BTCUSDT"), Some(&1));
        assert_eq!(map.get("ETHUSDT"), Some(&2));
        assert_eq!(map.get("XRPUSDT"), None);
    }
}
