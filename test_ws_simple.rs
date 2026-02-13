// Simple WebSocket test to debug Binance Futures connection
// Run with: cargo run --bin test_ws_simple

use futures_util::StreamExt;
use tokio_tungstenite::connect_async;

#[tokio::main]
async fn main() {
    // Test with just 2 symbols
    let symbols = vec!["btcusdt", "ethusdt"];

    // Create URL with bookTicker (correct case from official example)
    let streams: Vec<String> = symbols
        .iter()
        .map(|s| format!("{}@bookTicker", s))
        .collect();

    let url = format!("wss://fstream.binance.com/stream?streams={}", streams.join("/"));

    println!("[TEST] Connecting to: {}", url);
    println!("[TEST] Streams: {:?}", streams);

    match connect_async(&url).await {
        Ok((ws_stream, response)) => {
            println!("[TEST] ✅ Connected successfully!");
            println!("[TEST] Response status: {:?}", response.status());
            println!("[TEST] Response headers: {:?}", response.headers());

            let (_write, mut read) = ws_stream.split();

            println!("[TEST] Waiting for messages...");

            let mut count = 0;
            while let Some(msg) = read.next().await {
                match msg {
                    Ok(tokio_tungstenite::tungstenite::Message::Text(text)) => {
                        count += 1;
                        println!("\n[TEST] Message #{}: {}", count, text);

                        // Try to parse as JSON
                        match serde_json::from_str::<serde_json::Value>(&text) {
                            Ok(json) => {
                                println!("[TEST] Parsed JSON successfully!");
                                println!("[TEST] Structure: {}", serde_json::to_string_pretty(&json).unwrap());
                            }
                            Err(e) => {
                                println!("[TEST] ❌ Failed to parse JSON: {}", e);
                            }
                        }

                        if count >= 5 {
                            println!("\n[TEST] ✅ Received 5 messages, test successful!");
                            break;
                        }
                    }
                    Ok(tokio_tungstenite::tungstenite::Message::Ping(_)) => {
                        println!("[TEST] Received PING");
                    }
                    Ok(tokio_tungstenite::tungstenite::Message::Pong(_)) => {
                        println!("[TEST] Received PONG");
                    }
                    Ok(tokio_tungstenite::tungstenite::Message::Close(frame)) => {
                        println!("[TEST] ❌ Connection closed: {:?}", frame);
                        break;
                    }
                    Err(e) => {
                        println!("[TEST] ❌ Error: {}", e);
                        break;
                    }
                    _ => {}
                }
            }
        }
        Err(e) => {
            println!("[TEST] ❌ Connection failed: {}", e);
        }
    }
}
