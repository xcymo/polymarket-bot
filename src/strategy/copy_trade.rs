//! Copy trading - follow top traders
//!
//! Monitor successful traders' positions and copy their trades.

use crate::error::Result;
use crate::types::{Side, Signal};
use chrono::{DateTime, Utc};
use reqwest::Client;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Top trader to follow
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopTrader {
    /// Polymarket username
    pub username: String,
    /// Wallet address (Polygon)
    pub address: Option<String>,
    /// Win rate (0.0 - 1.0)
    pub win_rate: f64,
    /// Total profit
    pub total_profit: Decimal,
    /// Trust weight (how much to follow)
    pub weight: f64,
    /// Last updated
    pub updated_at: DateTime<Utc>,
}

/// Copy trade monitor
pub struct CopyTrader {
    http: Client,
    /// Traders to follow
    traders: Vec<TopTrader>,
    /// Known positions: trader_address -> market_id -> position
    known_positions: HashMap<String, HashMap<String, Position>>,
    /// Copy ratio (0.0 - 1.0, how much of their position to copy)
    copy_ratio: f64,
    /// Minimum trader profit to follow a trade
    #[allow(dead_code)]
    min_trader_profit: Decimal,
}

#[derive(Debug, Clone)]
struct Position {
    #[allow(dead_code)]
    market_id: String,
    token_id: String,
    side: Side,
    size: Decimal,
    #[allow(dead_code)]
    entry_price: Decimal,
    #[allow(dead_code)]
    timestamp: DateTime<Utc>,
}

impl CopyTrader {
    pub fn new() -> Self {
        Self {
            http: Client::new(),
            traders: Vec::new(),
            known_positions: HashMap::new(),
            copy_ratio: 0.5,  // Copy 50% of their position
            min_trader_profit: Decimal::new(1000, 0),  // $1000 minimum profit
        }
    }

    pub fn with_copy_ratio(mut self, ratio: f64) -> Self {
        self.copy_ratio = ratio.clamp(0.1, 1.0);
        self
    }

    /// Add a trader to follow
    pub fn add_trader(&mut self, trader: TopTrader) {
        // Remove if already exists
        self.traders.retain(|t| t.username != trader.username);
        self.traders.push(trader);
    }

    /// Add trader by username (will need to resolve address)
    pub async fn add_trader_by_username(&mut self, username: &str) -> Result<()> {
        // Try to get trader info from Polymarket
        // Note: This may require authentication or scraping
        let trader = TopTrader {
            username: username.to_string(),
            address: None,  // Will be resolved later
            win_rate: 0.0,
            total_profit: Decimal::ZERO,
            weight: 1.0,
            updated_at: Utc::now(),
        };
        
        self.add_trader(trader);
        tracing::info!("Added trader to follow: {}", username);
        Ok(())
    }

    /// Get signals from trader activity
    /// Returns new positions that we should copy
    pub async fn check_for_signals(&mut self) -> Result<Vec<CopySignal>> {
        let mut signals = Vec::new();

        for trader in &self.traders {
            if let Some(address) = &trader.address {
                // Check for new positions
                match self.get_trader_positions(address).await {
                    Ok(positions) => {
                        let known = self.known_positions
                            .entry(address.clone())
                            .or_default();

                        for pos in positions {
                            // Check if this is a new position
                            if !known.contains_key(&pos.market_id) {
                                // New position - generate copy signal
                                signals.push(CopySignal {
                                    trader: trader.clone(),
                                    market_id: pos.market_id.clone(),
                                    token_id: pos.token_id.clone(),
                                    side: pos.side,
                                    trader_size: pos.size,
                                    suggested_size: pos.size * Decimal::try_from(self.copy_ratio).unwrap_or(Decimal::new(5, 1)),
                                    timestamp: Utc::now(),
                                });

                                tracing::info!(
                                    "🎯 Copy signal: {} {} in {} (size: ${})",
                                    match pos.side { Side::Buy => "BUY", Side::Sell => "SELL" },
                                    trader.username,
                                    pos.market_id,
                                    pos.size
                                );
                            }

                            known.insert(pos.market_id.clone(), pos);
                        }

                        // Check for closed positions (exits)
                        let _current_markets: std::collections::HashSet<_> = 
                            known.keys().cloned().collect();
                        
                        // TODO: Detect exits and generate sell signals
                    }
                    Err(e) => {
                        tracing::warn!("Failed to get positions for {}: {}", trader.username, e);
                    }
                }
            }
        }

        Ok(signals)
    }

    /// Get trader's current positions from chain
    async fn get_trader_positions(&self, address: &str) -> Result<Vec<Position>> {
        // Query Polymarket API for user positions
        // Using their public profile endpoint
        
        let url = format!(
            "https://clob.polymarket.com/positions?user={}",
            address
        );
        
        match self.http.get(&url).send().await {
            Ok(resp) => {
                if resp.status().is_success() {
                    if let Ok(data) = resp.json::<serde_json::Value>().await {
                        let positions = self.parse_positions(&data);
                        return Ok(positions);
                    }
                }
                Ok(Vec::new())
            }
            Err(e) => {
                tracing::debug!("Failed to fetch positions for {}: {}", address, e);
                Ok(Vec::new())
            }
        }
    }

    fn parse_positions(&self, data: &serde_json::Value) -> Vec<Position> {
        let mut positions = Vec::new();
        
        if let Some(arr) = data.as_array() {
            for item in arr {
                if let (Some(asset), Some(size_str)) = (
                    item.get("asset").and_then(|v| v.as_str()),
                    item.get("size").and_then(|v| v.as_str()),
                ) {
                    if let Ok(size) = size_str.parse::<Decimal>() {
                        if size > Decimal::ZERO {
                            positions.push(Position {
                                market_id: item.get("market")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string(),
                                token_id: asset.to_string(),
                                side: Side::Buy,  // Holding = bought
                                size,
                                entry_price: Decimal::ZERO,  // Unknown from positions API
                                timestamp: Utc::now(),
                            });
                        }
                    }
                }
            }
        }
        
        positions
    }

    /// Resolve username to wallet address via Polymarket API
    pub async fn resolve_address(&self, username: &str) -> Result<Option<String>> {
        // Try Polymarket's user lookup
        let url = format!(
            "https://gamma-api.polymarket.com/users?username={}",
            username
        );
        
        match self.http.get(&url).send().await {
            Ok(resp) => {
                if resp.status().is_success() {
                    if let Ok(data) = resp.json::<serde_json::Value>().await {
                        if let Some(address) = data.get("proxyWallet")
                            .or_else(|| data.get("address"))
                            .and_then(|v| v.as_str()) 
                        {
                            return Ok(Some(address.to_string()));
                        }
                    }
                }
                Ok(None)
            }
            Err(_) => Ok(None)
        }
    }

    /// Get leaderboard data
    pub async fn get_leaderboard(&self) -> Result<Vec<TopTrader>> {
        let url = "https://gamma-api.polymarket.com/leaderboard?limit=50";
        
        match self.http.get(url).send().await {
            Ok(resp) => {
                if resp.status().is_success() {
                    if let Ok(data) = resp.json::<serde_json::Value>().await {
                        return Ok(self.parse_leaderboard(&data));
                    }
                }
                Ok(Vec::new())
            }
            Err(_) => Ok(Vec::new())
        }
    }

    fn parse_leaderboard(&self, data: &serde_json::Value) -> Vec<TopTrader> {
        let mut traders = Vec::new();
        
        if let Some(arr) = data.as_array() {
            for item in arr {
                if let Some(username) = item.get("username").and_then(|v| v.as_str()) {
                    let profit = item.get("pnl")
                        .and_then(|v| v.as_str())
                        .and_then(|s| s.parse::<Decimal>().ok())
                        .unwrap_or(Decimal::ZERO);
                    
                    let win_rate = item.get("winRate")
                        .and_then(|v| v.as_f64())
                        .unwrap_or(0.5);
                    
                    traders.push(TopTrader {
                        username: username.to_string(),
                        address: item.get("address").and_then(|v| v.as_str()).map(String::from),
                        win_rate,
                        total_profit: profit,
                        weight: 1.0,
                        updated_at: Utc::now(),
                    });
                }
            }
        }
        
        traders
    }
}

/// Signal to copy a trader's position
#[derive(Debug, Clone)]
pub struct CopySignal {
    pub trader: TopTrader,
    pub market_id: String,
    pub token_id: String,
    pub side: Side,
    pub trader_size: Decimal,
    pub suggested_size: Decimal,
    pub timestamp: DateTime<Utc>,
}

impl CopySignal {
    /// Convert to standard Signal for execution
    pub fn to_signal(&self, market_probability: Decimal) -> Signal {
        Signal {
            market_id: self.market_id.clone(),
            token_id: self.token_id.clone(),
            side: self.side,
            model_probability: market_probability, // Use current market price
            market_probability,
            edge: Decimal::ZERO, // We're copying, not analyzing
            confidence: Decimal::try_from(self.trader.win_rate).unwrap_or(Decimal::new(7, 1)),
            suggested_size: self.suggested_size,
            timestamp: self.timestamp,
        }
    }
}

/// Configuration for copy trading
#[derive(Debug, Clone, Deserialize)]
pub struct CopyTradeConfig {
    /// Enable copy trading
    #[serde(default)]
    pub enabled: bool,
    /// Usernames to follow
    #[serde(default)]
    pub follow_users: Vec<String>,
    /// Wallet addresses to follow
    #[serde(default)]
    pub follow_addresses: Vec<String>,
    /// Copy ratio (0.0 - 1.0)
    #[serde(default = "default_copy_ratio")]
    pub copy_ratio: f64,
    /// Delay before copying (seconds)
    #[serde(default)]
    pub delay_secs: u64,
}

fn default_copy_ratio() -> f64 {
    0.5
}

impl Default for CopyTradeConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            follow_users: Vec::new(),
            follow_addresses: Vec::new(),
            copy_ratio: 0.5,
            delay_secs: 0,
        }
    }
}
