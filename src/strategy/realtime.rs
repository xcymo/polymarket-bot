//! Real-time crypto trading strategy using WebSocket streams
//!
//! Combines Binance price stream with Polymarket orderbook for better predictions.

use crate::error::Result;
use crate::types::{Market, Side, Signal};
use chrono::Utc;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, RwLock};
use tracing::{debug, info, warn};

/// Real-time price data from Binance
#[derive(Debug, Clone)]
pub struct RealtimePrice {
    pub symbol: String,
    pub price: Decimal,
    pub change_1m: Decimal,  // 1-minute change %
    pub change_5m: Decimal,  // 5-minute change %
    pub volume: Decimal,
    pub timestamp: Instant,
}

/// Real-time trading engine
pub struct RealtimeEngine {
    /// Current prices (shared state)
    prices: Arc<RwLock<HashMap<String, RealtimePrice>>>,
    /// Price history for momentum calculation
    history: Arc<RwLock<HashMap<String, Vec<(Instant, Decimal)>>>>,
    /// Minimum momentum to trade (%)
    min_momentum: Decimal,
    /// Signal output channel
    signal_tx: mpsc::Sender<Signal>,
}

impl RealtimeEngine {
    pub fn new(signal_tx: mpsc::Sender<Signal>) -> Self {
        Self {
            prices: Arc::new(RwLock::new(HashMap::new())),
            history: Arc::new(RwLock::new(HashMap::new())),
            min_momentum: dec!(0.02), // 0.02% minimum
            signal_tx,
        }
    }

    /// Update price from WebSocket stream
    pub async fn update_price(&self, symbol: &str, price: Decimal) {
        let now = Instant::now();
        
        // Update history
        {
            let mut history = self.history.write().await;
            let entries = history.entry(symbol.to_string()).or_insert_with(Vec::new);
            entries.push((now, price));
            
            // Keep only last 10 minutes of data
            let cutoff = now - Duration::from_secs(600);
            entries.retain(|(t, _)| *t > cutoff);
        }

        // Calculate momentum
        let (change_1m, change_5m) = {
            let history = self.history.read().await;
            if let Some(entries) = history.get(symbol) {
                let change_1m = Self::calc_change(entries, now, 60);
                let change_5m = Self::calc_change(entries, now, 300);
                (change_1m, change_5m)
            } else {
                (Decimal::ZERO, Decimal::ZERO)
            }
        };

        // Update current price
        {
            let mut prices = self.prices.write().await;
            prices.insert(symbol.to_string(), RealtimePrice {
                symbol: symbol.to_string(),
                price,
                change_1m,
                change_5m,
                volume: Decimal::ZERO,
                timestamp: now,
            });
        }

        // Log significant moves
        if change_1m.abs() > dec!(0.1) {
            info!("📊 {} ${:.2} | 1m: {:.3}% | 5m: {:.3}%", 
                symbol, price, change_1m, change_5m);
        }
    }

    fn calc_change(entries: &[(Instant, Decimal)], now: Instant, secs: u64) -> Decimal {
        let cutoff = now - Duration::from_secs(secs);
        let old_price = entries.iter()
            .filter(|(t, _)| *t <= cutoff)
            .last()
            .or_else(|| entries.first())
            .map(|(_, p)| *p);
        
        let new_price = entries.last().map(|(_, p)| *p);
        
        match (old_price, new_price) {
            (Some(old), Some(new)) if old > Decimal::ZERO => {
                (new - old) / old * dec!(100)
            }
            _ => Decimal::ZERO,
        }
    }

    /// Get current price for symbol
    pub async fn get_price(&self, symbol: &str) -> Option<RealtimePrice> {
        self.prices.read().await.get(symbol).cloned()
    }

    /// Generate signal for crypto market based on real-time data
    pub async fn generate_signal(&self, market: &Market) -> Option<Signal> {
        // Detect which crypto this market is for
        let symbol = Self::detect_crypto(&market.question)?;
        
        let price_data = self.get_price(&symbol).await?;
        
        // Use 1-minute momentum for short-term markets
        let momentum = price_data.change_1m;
        
        if momentum.abs() < self.min_momentum {
            debug!("{}: momentum {:.4}% below threshold", symbol, momentum);
            return None;
        }

        // Determine direction
        let (direction, model_prob) = if momentum > Decimal::ZERO {
            ("Up", dec!(0.5) + (momentum * dec!(2)).min(dec!(0.4)))
        } else {
            ("Down", dec!(0.5) + (momentum.abs() * dec!(2)).min(dec!(0.4)))
        };

        // Find the matching outcome
        let outcome = market.outcomes.iter()
            .find(|o| o.outcome.to_lowercase() == direction.to_lowercase())?;
        
        let market_prob = outcome.price;
        let edge = model_prob - market_prob;
        
        if edge < dec!(0.03) {
            debug!("{}: edge {:.2}% too small", symbol, edge * dec!(100));
            return None;
        }

        info!("🎯 {} {}: momentum {:.3}% → {} | edge {:.1}%", 
            symbol, direction, momentum, market.question.chars().take(30).collect::<String>(), 
            edge * dec!(100));

        Some(Signal {
            market_id: market.id.clone(),
            token_id: outcome.token_id.clone(),
            side: Side::Buy,
            model_probability: model_prob,
            market_probability: market_prob,
            edge,
            confidence: dec!(0.7),
            suggested_size: dec!(0.1), // 10% of portfolio
            timestamp: Utc::now(),
        })
    }

    fn detect_crypto(question: &str) -> Option<String> {
        let q = question.to_lowercase();
        if q.contains("bitcoin") || q.contains("btc") {
            Some("BTCUSDT".to_string())
        } else if q.contains("ethereum") || q.contains("eth") {
            Some("ETHUSDT".to_string())
        } else if q.contains("solana") || q.contains("sol") {
            Some("SOLUSDT".to_string())
        } else if q.contains("xrp") {
            Some("XRPUSDT".to_string())
        } else {
            None
        }
    }
}

/// Start Binance WebSocket and feed into engine
pub async fn start_binance_feed(engine: Arc<RealtimeEngine>) -> Result<()> {
    use futures_util::StreamExt;
    use tokio_tungstenite::connect_async;

    let symbols = vec!["btcusdt", "ethusdt", "solusdt", "xrpusdt"];
    let streams: Vec<String> = symbols.iter().map(|s| format!("{}@trade", s)).collect();
    let url = format!("wss://stream.binance.com:9443/stream?streams={}", streams.join("/"));

    info!("🔌 Connecting to Binance WebSocket...");
    
    loop {
        match connect_async(&url).await {
            Ok((ws_stream, _)) => {
                info!("✅ Connected to Binance WebSocket");
                let (_, mut read) = ws_stream.split();
                
                while let Some(msg) = read.next().await {
                    if let Ok(tokio_tungstenite::tungstenite::Message::Text(text)) = msg {
                        if let Ok(data) = serde_json::from_str::<serde_json::Value>(&text) {
                            if let Some(trade) = data.get("data") {
                                let symbol = trade["s"].as_str().unwrap_or("");
                                if let Some(price) = trade["p"].as_str()
                                    .and_then(|p| p.parse::<Decimal>().ok()) 
                                {
                                    engine.update_price(symbol, price).await;
                                }
                            }
                        }
                    }
                }
                warn!("WebSocket disconnected, reconnecting...");
            }
            Err(e) => {
                warn!("WebSocket connection failed: {}, retrying in 5s...", e);
            }
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}
