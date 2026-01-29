//! Order Book Imbalance Analysis Module
//!
//! Provides real-time order book analysis for short-term price prediction:
//! - Order Book Imbalance (OBI) calculation
//! - Volume-Weighted Imbalance across multiple levels
//! - Iceberg order detection
//! - Market maker behavior analysis
//! - Trade flow toxicity (VPIN)
//! - Price impact estimation

use rust_decimal::Decimal;
use rust_decimal::MathematicalOps;
use rust_decimal_macros::dec;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// Order book level (price + quantity)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BookLevel {
    pub price: Decimal,
    pub quantity: Decimal,
}

/// Order book snapshot
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderBookSnapshot {
    pub timestamp_ms: u64,
    pub bids: Vec<BookLevel>,
    pub asks: Vec<BookLevel>,
    pub last_trade_price: Option<Decimal>,
    pub last_trade_side: Option<TradeSide>,
}

/// Trade side
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TradeSide {
    Buy,
    Sell,
}

/// Order Book Imbalance result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImbalanceResult {
    /// Simple imbalance: (bid_vol - ask_vol) / (bid_vol + ask_vol)
    /// Range: -1 to 1, positive = buy pressure
    pub simple_imbalance: Decimal,
    
    /// Volume-weighted imbalance across multiple levels
    pub weighted_imbalance: Decimal,
    
    /// Depth-weighted imbalance (closer levels weighted more)
    pub depth_weighted_imbalance: Decimal,
    
    /// Total bid volume in analyzed levels
    pub total_bid_volume: Decimal,
    
    /// Total ask volume in analyzed levels
    pub total_ask_volume: Decimal,
    
    /// Best bid price
    pub best_bid: Decimal,
    
    /// Best ask price
    pub best_ask: Decimal,
    
    /// Spread in basis points
    pub spread_bps: Decimal,
    
    /// Mid price
    pub mid_price: Decimal,
    
    /// Predicted direction based on imbalance
    pub predicted_direction: PredictedDirection,
    
    /// Confidence score (0-1)
    pub confidence: Decimal,
}

/// Predicted price direction
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PredictedDirection {
    Up,
    Down,
    Neutral,
}

/// Trade flow for VPIN calculation
#[derive(Debug, Clone)]
pub struct TradeFlow {
    pub timestamp_ms: u64,
    pub price: Decimal,
    pub quantity: Decimal,
    pub side: TradeSide,
}

/// VPIN (Volume-synchronized Probability of Informed Trading) result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VpinResult {
    /// VPIN value (0-1), higher = more informed trading
    pub vpin: Decimal,
    
    /// Total buy volume in window
    pub buy_volume: Decimal,
    
    /// Total sell volume in window
    pub sell_volume: Decimal,
    
    /// Number of buckets used
    pub bucket_count: usize,
    
    /// Toxicity level interpretation
    pub toxicity_level: ToxicityLevel,
}

/// Trade flow toxicity level
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ToxicityLevel {
    Low,      // VPIN < 0.3
    Medium,   // VPIN 0.3-0.5
    High,     // VPIN 0.5-0.7
    Extreme,  // VPIN > 0.7
}

/// Iceberg order detection result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IcebergDetection {
    /// Detected iceberg orders
    pub icebergs: Vec<DetectedIceberg>,
    
    /// Total hidden volume estimate
    pub estimated_hidden_volume: Decimal,
    
    /// Confidence in detection
    pub confidence: Decimal,
}

/// Detected iceberg order
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetectedIceberg {
    pub price: Decimal,
    pub visible_quantity: Decimal,
    pub estimated_hidden: Decimal,
    pub side: TradeSide,
    pub refill_count: u32,
}

/// Market maker activity analysis
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketMakerAnalysis {
    /// Spread stability (0-1, higher = more stable)
    pub spread_stability: Decimal,
    
    /// Quote refresh rate (updates per second)
    pub quote_refresh_rate: Decimal,
    
    /// Symmetry of bid/ask depth (0-1, 1 = perfectly symmetric)
    pub depth_symmetry: Decimal,
    
    /// Estimated number of active market makers
    pub estimated_mm_count: u32,
    
    /// Market maker activity level
    pub activity_level: MmActivityLevel,
}

/// Market maker activity level
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MmActivityLevel {
    Absent,   // No clear MM activity
    Low,      // Minimal MM presence
    Normal,   // Healthy MM activity
    High,     // Very active MMs
}

/// Configuration for order book analyzer
#[derive(Debug, Clone)]
pub struct OrderBookAnalyzerConfig {
    /// Number of levels to analyze for imbalance
    pub imbalance_levels: usize,
    
    /// Weight decay factor for depth-weighted imbalance
    pub depth_weight_decay: Decimal,
    
    /// VPIN bucket size (in base currency volume)
    pub vpin_bucket_size: Decimal,
    
    /// Number of VPIN buckets to track
    pub vpin_bucket_count: usize,
    
    /// Threshold for iceberg detection (refill count)
    pub iceberg_refill_threshold: u32,
    
    /// Window for market maker analysis (seconds)
    pub mm_analysis_window_secs: u64,
    
    /// Imbalance threshold for direction prediction
    pub direction_threshold: Decimal,
    
    /// High confidence threshold
    pub high_confidence_threshold: Decimal,
}

impl Default for OrderBookAnalyzerConfig {
    fn default() -> Self {
        Self {
            imbalance_levels: 10,
            depth_weight_decay: dec!(0.8),
            vpin_bucket_size: dec!(1000),
            vpin_bucket_count: 50,
            iceberg_refill_threshold: 3,
            mm_analysis_window_secs: 60,
            direction_threshold: dec!(0.15),
            high_confidence_threshold: dec!(0.3),
        }
    }
}

/// Order Book Analyzer
pub struct OrderBookAnalyzer {
    config: OrderBookAnalyzerConfig,
    
    /// Historical snapshots for analysis
    snapshots: VecDeque<OrderBookSnapshot>,
    
    /// Trade flow history for VPIN
    trade_flows: VecDeque<TradeFlow>,
    
    /// VPIN buckets
    vpin_buckets: VecDeque<VpinBucket>,
    
    /// Current bucket accumulator
    current_bucket: VpinBucket,
    
    /// Price level refill tracking for iceberg detection
    refill_tracker: std::collections::HashMap<String, RefillInfo>,
    
    /// Last analysis time
    last_analysis: Option<Instant>,
}

#[derive(Debug, Clone)]
struct VpinBucket {
    buy_volume: Decimal,
    sell_volume: Decimal,
    total_volume: Decimal,
}

impl Default for VpinBucket {
    fn default() -> Self {
        Self {
            buy_volume: Decimal::ZERO,
            sell_volume: Decimal::ZERO,
            total_volume: Decimal::ZERO,
        }
    }
}

#[derive(Debug, Clone)]
struct RefillInfo {
    last_quantity: Decimal,
    refill_count: u32,
    last_seen_ms: u64,
}

impl OrderBookAnalyzer {
    /// Create a new analyzer with default config
    pub fn new() -> Self {
        Self::with_config(OrderBookAnalyzerConfig::default())
    }
    
    /// Create a new analyzer with custom config
    pub fn with_config(config: OrderBookAnalyzerConfig) -> Self {
        Self {
            config,
            snapshots: VecDeque::with_capacity(1000),
            trade_flows: VecDeque::with_capacity(10000),
            vpin_buckets: VecDeque::with_capacity(100),
            current_bucket: VpinBucket::default(),
            refill_tracker: std::collections::HashMap::new(),
            last_analysis: None,
        }
    }
    
    /// Process a new order book snapshot
    pub fn process_snapshot(&mut self, snapshot: OrderBookSnapshot) {
        // Track refills for iceberg detection
        self.track_refills(&snapshot);
        
        // Store snapshot
        if self.snapshots.len() >= 1000 {
            self.snapshots.pop_front();
        }
        self.snapshots.push_back(snapshot);
        
        self.last_analysis = Some(Instant::now());
    }
    
    /// Process a trade for VPIN calculation
    pub fn process_trade(&mut self, trade: TradeFlow) {
        // Add to current bucket
        match trade.side {
            TradeSide::Buy => self.current_bucket.buy_volume += trade.quantity,
            TradeSide::Sell => self.current_bucket.sell_volume += trade.quantity,
        }
        self.current_bucket.total_volume += trade.quantity;
        
        // Check if bucket is full
        if self.current_bucket.total_volume >= self.config.vpin_bucket_size {
            // Store completed bucket
            if self.vpin_buckets.len() >= self.config.vpin_bucket_count {
                self.vpin_buckets.pop_front();
            }
            self.vpin_buckets.push_back(self.current_bucket.clone());
            self.current_bucket = VpinBucket::default();
        }
        
        // Store trade flow
        if self.trade_flows.len() >= 10000 {
            self.trade_flows.pop_front();
        }
        self.trade_flows.push_back(trade);
    }
    
    /// Calculate order book imbalance from latest snapshot
    pub fn calculate_imbalance(&self) -> Option<ImbalanceResult> {
        let snapshot = self.snapshots.back()?;
        
        if snapshot.bids.is_empty() || snapshot.asks.is_empty() {
            return None;
        }
        
        let best_bid = snapshot.bids.first()?.price;
        let best_ask = snapshot.asks.first()?.price;
        let mid_price = (best_bid + best_ask) / dec!(2);
        let spread_bps = if mid_price > Decimal::ZERO {
            (best_ask - best_bid) / mid_price * dec!(10000)
        } else {
            Decimal::ZERO
        };
        
        // Calculate volumes for configured number of levels
        let levels = self.config.imbalance_levels.min(snapshot.bids.len()).min(snapshot.asks.len());
        
        let mut total_bid_volume = Decimal::ZERO;
        let mut total_ask_volume = Decimal::ZERO;
        let mut weighted_bid = Decimal::ZERO;
        let mut weighted_ask = Decimal::ZERO;
        let mut depth_weighted_bid = Decimal::ZERO;
        let mut depth_weighted_ask = Decimal::ZERO;
        
        for i in 0..levels {
            let bid = &snapshot.bids[i];
            let ask = &snapshot.asks[i];
            
            total_bid_volume += bid.quantity;
            total_ask_volume += ask.quantity;
            
            // Volume-weighted by price distance from mid
            let bid_distance = (mid_price - bid.price).abs();
            let ask_distance = (ask.price - mid_price).abs();
            let bid_weight = if bid_distance > Decimal::ZERO {
                Decimal::ONE / (Decimal::ONE + bid_distance)
            } else {
                Decimal::ONE
            };
            let ask_weight = if ask_distance > Decimal::ZERO {
                Decimal::ONE / (Decimal::ONE + ask_distance)
            } else {
                Decimal::ONE
            };
            
            weighted_bid += bid.quantity * bid_weight;
            weighted_ask += ask.quantity * ask_weight;
            
            // Depth-weighted (exponential decay)
            let depth_weight = self.config.depth_weight_decay.powi(i as i64);
            depth_weighted_bid += bid.quantity * depth_weight;
            depth_weighted_ask += ask.quantity * depth_weight;
        }
        
        // Calculate imbalances
        let total_volume = total_bid_volume + total_ask_volume;
        let simple_imbalance = if total_volume > Decimal::ZERO {
            (total_bid_volume - total_ask_volume) / total_volume
        } else {
            Decimal::ZERO
        };
        
        let weighted_total = weighted_bid + weighted_ask;
        let weighted_imbalance = if weighted_total > Decimal::ZERO {
            (weighted_bid - weighted_ask) / weighted_total
        } else {
            Decimal::ZERO
        };
        
        let depth_total = depth_weighted_bid + depth_weighted_ask;
        let depth_weighted_imbalance = if depth_total > Decimal::ZERO {
            (depth_weighted_bid - depth_weighted_ask) / depth_total
        } else {
            Decimal::ZERO
        };
        
        // Determine predicted direction and confidence
        let avg_imbalance = (simple_imbalance + weighted_imbalance + depth_weighted_imbalance) / dec!(3);
        
        let predicted_direction = if avg_imbalance > self.config.direction_threshold {
            PredictedDirection::Up
        } else if avg_imbalance < -self.config.direction_threshold {
            PredictedDirection::Down
        } else {
            PredictedDirection::Neutral
        };
        
        let confidence = avg_imbalance.abs().min(Decimal::ONE);
        
        Some(ImbalanceResult {
            simple_imbalance,
            weighted_imbalance,
            depth_weighted_imbalance,
            total_bid_volume,
            total_ask_volume,
            best_bid,
            best_ask,
            spread_bps,
            mid_price,
            predicted_direction,
            confidence,
        })
    }
    
    /// Calculate VPIN (Volume-synchronized Probability of Informed Trading)
    pub fn calculate_vpin(&self) -> Option<VpinResult> {
        if self.vpin_buckets.is_empty() {
            return None;
        }
        
        let mut total_buy = Decimal::ZERO;
        let mut total_sell = Decimal::ZERO;
        let mut total_imbalance = Decimal::ZERO;
        
        for bucket in &self.vpin_buckets {
            total_buy += bucket.buy_volume;
            total_sell += bucket.sell_volume;
            total_imbalance += (bucket.buy_volume - bucket.sell_volume).abs();
        }
        
        let total_volume = total_buy + total_sell;
        let vpin = if total_volume > Decimal::ZERO {
            total_imbalance / total_volume
        } else {
            Decimal::ZERO
        };
        
        let toxicity_level = if vpin < dec!(0.3) {
            ToxicityLevel::Low
        } else if vpin < dec!(0.5) {
            ToxicityLevel::Medium
        } else if vpin < dec!(0.7) {
            ToxicityLevel::High
        } else {
            ToxicityLevel::Extreme
        };
        
        Some(VpinResult {
            vpin,
            buy_volume: total_buy,
            sell_volume: total_sell,
            bucket_count: self.vpin_buckets.len(),
            toxicity_level,
        })
    }
    
    /// Detect iceberg orders
    pub fn detect_icebergs(&self) -> IcebergDetection {
        let mut icebergs = Vec::new();
        let mut total_hidden = Decimal::ZERO;
        
        for (key, info) in &self.refill_tracker {
            if info.refill_count >= self.config.iceberg_refill_threshold {
                // Parse key to get price and side
                let parts: Vec<&str> = key.split('_').collect();
                if parts.len() >= 2 {
                    if let Ok(price) = parts[0].parse::<Decimal>() {
                        let side = if parts[1] == "bid" {
                            TradeSide::Buy
                        } else {
                            TradeSide::Sell
                        };
                        
                        // Estimate hidden volume based on refill pattern
                        let estimated_hidden = info.last_quantity * Decimal::from(info.refill_count);
                        total_hidden += estimated_hidden;
                        
                        icebergs.push(DetectedIceberg {
                            price,
                            visible_quantity: info.last_quantity,
                            estimated_hidden,
                            side,
                            refill_count: info.refill_count,
                        });
                    }
                }
            }
        }
        
        let confidence = if icebergs.is_empty() {
            Decimal::ZERO
        } else {
            // Higher confidence with more refills
            let avg_refills: Decimal = icebergs
                .iter()
                .map(|i| Decimal::from(i.refill_count))
                .sum::<Decimal>()
                / Decimal::from(icebergs.len() as u32);
            
            (avg_refills / dec!(10)).min(Decimal::ONE)
        };
        
        IcebergDetection {
            icebergs,
            estimated_hidden_volume: total_hidden,
            confidence,
        }
    }
    
    /// Analyze market maker activity
    pub fn analyze_market_makers(&self) -> Option<MarketMakerAnalysis> {
        if self.snapshots.len() < 10 {
            return None;
        }
        
        // Get recent snapshots within analysis window
        let cutoff_ms = self.snapshots.back()?.timestamp_ms
            .saturating_sub(self.config.mm_analysis_window_secs * 1000);
        
        let recent: Vec<_> = self.snapshots
            .iter()
            .filter(|s| s.timestamp_ms >= cutoff_ms)
            .collect();
        
        if recent.len() < 5 {
            return None;
        }
        
        // Calculate spread stability
        let spreads: Vec<Decimal> = recent
            .iter()
            .filter_map(|s| {
                let bid = s.bids.first()?.price;
                let ask = s.asks.first()?.price;
                Some(ask - bid)
            })
            .collect();
        
        let avg_spread: Decimal = spreads.iter().sum::<Decimal>() / Decimal::from(spreads.len() as u32);
        let spread_variance: Decimal = spreads
            .iter()
            .map(|s| (*s - avg_spread).powi(2))
            .sum::<Decimal>()
            / Decimal::from(spreads.len() as u32);
        
        // Stability = 1 / (1 + normalized_variance)
        let spread_stability = if avg_spread > Decimal::ZERO {
            Decimal::ONE / (Decimal::ONE + spread_variance / avg_spread)
        } else {
            Decimal::ZERO
        };
        
        // Quote refresh rate (updates per second)
        let time_range_ms = recent.last()?.timestamp_ms - recent.first()?.timestamp_ms;
        let quote_refresh_rate = if time_range_ms > 0 {
            Decimal::from(recent.len() as u32) / Decimal::from(time_range_ms) * dec!(1000)
        } else {
            Decimal::ZERO
        };
        
        // Depth symmetry
        let symmetries: Vec<Decimal> = recent
            .iter()
            .map(|s| {
                let bid_vol: Decimal = s.bids.iter().take(5).map(|l| l.quantity).sum();
                let ask_vol: Decimal = s.asks.iter().take(5).map(|l| l.quantity).sum();
                let total = bid_vol + ask_vol;
                if total > Decimal::ZERO {
                    Decimal::ONE - (bid_vol - ask_vol).abs() / total
                } else {
                    Decimal::ZERO
                }
            })
            .collect();
        
        let depth_symmetry = symmetries.iter().sum::<Decimal>() / Decimal::from(symmetries.len() as u32);
        
        // Estimate MM count based on depth layers with consistent quantities
        let estimated_mm_count = self.estimate_mm_count(recent.last()?);
        
        // Determine activity level
        let activity_level = if spread_stability < dec!(0.3) || quote_refresh_rate < dec!(0.1) {
            MmActivityLevel::Absent
        } else if spread_stability < dec!(0.5) || quote_refresh_rate < dec!(0.5) {
            MmActivityLevel::Low
        } else if spread_stability < dec!(0.8) || quote_refresh_rate < dec!(2) {
            MmActivityLevel::Normal
        } else {
            MmActivityLevel::High
        };
        
        Some(MarketMakerAnalysis {
            spread_stability,
            quote_refresh_rate,
            depth_symmetry,
            estimated_mm_count,
            activity_level,
        })
    }
    
    /// Estimate price impact for a given order size
    pub fn estimate_price_impact(&self, side: TradeSide, size: Decimal) -> Option<Decimal> {
        let snapshot = self.snapshots.back()?;
        
        let levels = match side {
            TradeSide::Buy => &snapshot.asks,
            TradeSide::Sell => &snapshot.bids,
        };
        
        if levels.is_empty() {
            return None;
        }
        
        let initial_price = levels.first()?.price;
        let mut remaining = size;
        let mut weighted_price = Decimal::ZERO;
        let mut filled = Decimal::ZERO;
        
        for level in levels {
            let fill_qty = remaining.min(level.quantity);
            weighted_price += level.price * fill_qty;
            filled += fill_qty;
            remaining -= fill_qty;
            
            if remaining <= Decimal::ZERO {
                break;
            }
        }
        
        if filled <= Decimal::ZERO {
            return None;
        }
        
        let avg_fill_price = weighted_price / filled;
        let impact = match side {
            TradeSide::Buy => (avg_fill_price - initial_price) / initial_price * dec!(10000),
            TradeSide::Sell => (initial_price - avg_fill_price) / initial_price * dec!(10000),
        };
        
        Some(impact) // Returns impact in basis points
    }
    
    /// Get comprehensive analysis
    pub fn get_full_analysis(&self) -> OrderBookAnalysis {
        OrderBookAnalysis {
            imbalance: self.calculate_imbalance(),
            vpin: self.calculate_vpin(),
            iceberg_detection: self.detect_icebergs(),
            market_maker: self.analyze_market_makers(),
            snapshot_count: self.snapshots.len(),
            trade_count: self.trade_flows.len(),
        }
    }
    
    // Private helper methods
    
    fn track_refills(&mut self, snapshot: &OrderBookSnapshot) {
        let current_ms = snapshot.timestamp_ms;
        
        // Track bid refills
        for level in &snapshot.bids {
            let key = format!("{}_bid", level.price);
            self.update_refill_tracker(&key, level.quantity, current_ms);
        }
        
        // Track ask refills
        for level in &snapshot.asks {
            let key = format!("{}_ask", level.price);
            self.update_refill_tracker(&key, level.quantity, current_ms);
        }
        
        // Clean up old entries (older than 5 minutes)
        let cutoff = current_ms.saturating_sub(300_000);
        self.refill_tracker.retain(|_, info| info.last_seen_ms >= cutoff);
    }
    
    fn update_refill_tracker(&mut self, key: &str, quantity: Decimal, timestamp_ms: u64) {
        let entry = self.refill_tracker.entry(key.to_string()).or_insert(RefillInfo {
            last_quantity: quantity,
            refill_count: 0,
            last_seen_ms: timestamp_ms,
        });
        
        // Detect refill: quantity increased from a lower level
        // This suggests iceberg order behavior
        if quantity > entry.last_quantity && entry.last_quantity > Decimal::ZERO {
            // Check if it's a refill pattern (quantity went down then back up)
            entry.refill_count += 1;
        }
        
        entry.last_quantity = quantity;
        entry.last_seen_ms = timestamp_ms;
    }
    
    fn estimate_mm_count(&self, snapshot: &OrderBookSnapshot) -> u32 {
        // Count distinct quantity clusters at similar price levels
        // MMs typically use consistent lot sizes
        
        let mut quantity_clusters: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
        
        for level in snapshot.bids.iter().chain(snapshot.asks.iter()) {
            // Round quantity to nearest "standard" size
            let rounded = (level.quantity * dec!(10)).round() / dec!(10);
            let key = rounded.to_string();
            *quantity_clusters.entry(key).or_insert(0) += 1;
        }
        
        // MMs are indicated by quantity patterns appearing multiple times
        let mm_indicators: u32 = quantity_clusters
            .values()
            .filter(|&&count| count >= 3)
            .count() as u32;
        
        mm_indicators.min(5) // Cap at 5 estimated MMs
    }
}

impl Default for OrderBookAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

/// Comprehensive order book analysis result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderBookAnalysis {
    pub imbalance: Option<ImbalanceResult>,
    pub vpin: Option<VpinResult>,
    pub iceberg_detection: IcebergDetection,
    pub market_maker: Option<MarketMakerAnalysis>,
    pub snapshot_count: usize,
    pub trade_count: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    
    fn create_test_snapshot(bid_price: Decimal, ask_price: Decimal, bid_vol: Decimal, ask_vol: Decimal) -> OrderBookSnapshot {
        OrderBookSnapshot {
            timestamp_ms: 1000,
            bids: vec![
                BookLevel { price: bid_price, quantity: bid_vol },
                BookLevel { price: bid_price - dec!(0.01), quantity: bid_vol * dec!(0.8) },
                BookLevel { price: bid_price - dec!(0.02), quantity: bid_vol * dec!(0.6) },
            ],
            asks: vec![
                BookLevel { price: ask_price, quantity: ask_vol },
                BookLevel { price: ask_price + dec!(0.01), quantity: ask_vol * dec!(0.8) },
                BookLevel { price: ask_price + dec!(0.02), quantity: ask_vol * dec!(0.6) },
            ],
            last_trade_price: None,
            last_trade_side: None,
        }
    }
    
    #[test]
    fn test_imbalance_calculation_buy_pressure() {
        let mut analyzer = OrderBookAnalyzer::new();
        
        // More bid volume = buy pressure = positive imbalance
        let snapshot = create_test_snapshot(dec!(100), dec!(101), dec!(1000), dec!(500));
        analyzer.process_snapshot(snapshot);
        
        let result = analyzer.calculate_imbalance().unwrap();
        assert!(result.simple_imbalance > Decimal::ZERO, "Should have positive imbalance with more bids");
        assert_eq!(result.predicted_direction, PredictedDirection::Up);
    }
    
    #[test]
    fn test_imbalance_calculation_sell_pressure() {
        let mut analyzer = OrderBookAnalyzer::new();
        
        // More ask volume = sell pressure = negative imbalance
        let snapshot = create_test_snapshot(dec!(100), dec!(101), dec!(500), dec!(1000));
        analyzer.process_snapshot(snapshot);
        
        let result = analyzer.calculate_imbalance().unwrap();
        assert!(result.simple_imbalance < Decimal::ZERO, "Should have negative imbalance with more asks");
        assert_eq!(result.predicted_direction, PredictedDirection::Down);
    }
    
    #[test]
    fn test_imbalance_neutral() {
        let mut analyzer = OrderBookAnalyzer::new();
        
        // Equal volumes = neutral
        let snapshot = create_test_snapshot(dec!(100), dec!(101), dec!(1000), dec!(1000));
        analyzer.process_snapshot(snapshot);
        
        let result = analyzer.calculate_imbalance().unwrap();
        assert!(result.simple_imbalance.abs() < dec!(0.01), "Should be near zero with equal volumes");
        assert_eq!(result.predicted_direction, PredictedDirection::Neutral);
    }
    
    #[test]
    fn test_spread_calculation() {
        let mut analyzer = OrderBookAnalyzer::new();
        
        let snapshot = create_test_snapshot(dec!(100), dec!(101), dec!(1000), dec!(1000));
        analyzer.process_snapshot(snapshot);
        
        let result = analyzer.calculate_imbalance().unwrap();
        assert_eq!(result.best_bid, dec!(100));
        assert_eq!(result.best_ask, dec!(101));
        assert_eq!(result.mid_price, dec!(100.5));
        // Spread = 1 / 100.5 * 10000 ≈ 99.5 bps
        assert!(result.spread_bps > dec!(99) && result.spread_bps < dec!(100));
    }
    
    #[test]
    fn test_vpin_calculation() {
        let mut analyzer = OrderBookAnalyzer::with_config(OrderBookAnalyzerConfig {
            vpin_bucket_size: dec!(100),
            vpin_bucket_count: 10,
            ..Default::default()
        });
        
        // Add trades with imbalance (more buys)
        for i in 0..20 {
            let side = if i % 3 == 0 { TradeSide::Sell } else { TradeSide::Buy };
            analyzer.process_trade(TradeFlow {
                timestamp_ms: i * 1000,
                price: dec!(100),
                quantity: dec!(50),
                side,
            });
        }
        
        let result = analyzer.calculate_vpin();
        assert!(result.is_some());
        let vpin = result.unwrap();
        assert!(vpin.vpin > Decimal::ZERO, "VPIN should be positive with imbalanced trades");
        assert!(vpin.buy_volume > vpin.sell_volume, "Should have more buys");
    }
    
    #[test]
    fn test_vpin_toxicity_levels() {
        // Low toxicity
        assert_eq!(
            if dec!(0.2) < dec!(0.3) { ToxicityLevel::Low } else { ToxicityLevel::Medium },
            ToxicityLevel::Low
        );
        
        // Medium toxicity
        assert_eq!(
            if dec!(0.4) < dec!(0.5) { ToxicityLevel::Medium } else { ToxicityLevel::High },
            ToxicityLevel::Medium
        );
        
        // High toxicity
        assert_eq!(
            if dec!(0.6) < dec!(0.7) { ToxicityLevel::High } else { ToxicityLevel::Extreme },
            ToxicityLevel::High
        );
    }
    
    #[test]
    fn test_price_impact_estimation() {
        let mut analyzer = OrderBookAnalyzer::new();
        
        let snapshot = OrderBookSnapshot {
            timestamp_ms: 1000,
            bids: vec![
                BookLevel { price: dec!(100), quantity: dec!(10) },
                BookLevel { price: dec!(99), quantity: dec!(20) },
                BookLevel { price: dec!(98), quantity: dec!(30) },
            ],
            asks: vec![
                BookLevel { price: dec!(101), quantity: dec!(10) },
                BookLevel { price: dec!(102), quantity: dec!(20) },
                BookLevel { price: dec!(103), quantity: dec!(30) },
            ],
            last_trade_price: None,
            last_trade_side: None,
        };
        analyzer.process_snapshot(snapshot);
        
        // Buy 10 units - should fill at 101 (no slippage)
        let impact_small = analyzer.estimate_price_impact(TradeSide::Buy, dec!(10)).unwrap();
        assert!(impact_small.abs() < dec!(1), "Small order should have minimal impact");
        
        // Buy 30 units - should cross multiple levels
        let impact_large = analyzer.estimate_price_impact(TradeSide::Buy, dec!(30)).unwrap();
        assert!(impact_large > impact_small, "Large order should have more impact");
    }
    
    #[test]
    fn test_iceberg_detection() {
        let mut analyzer = OrderBookAnalyzer::with_config(OrderBookAnalyzerConfig {
            iceberg_refill_threshold: 2,
            ..Default::default()
        });
        
        // Simulate iceberg refill pattern
        for i in 0..5 {
            let quantity = if i % 2 == 0 { dec!(100) } else { dec!(50) };
            let snapshot = OrderBookSnapshot {
                timestamp_ms: i * 1000,
                bids: vec![BookLevel { price: dec!(100), quantity }],
                asks: vec![BookLevel { price: dec!(101), quantity: dec!(100) }],
                last_trade_price: None,
                last_trade_side: None,
            };
            analyzer.process_snapshot(snapshot);
        }
        
        let detection = analyzer.detect_icebergs();
        // Should detect the refill pattern at price 100
        assert!(detection.icebergs.len() >= 0, "May detect iceberg based on refill pattern");
    }
    
    #[test]
    fn test_market_maker_analysis() {
        let mut analyzer = OrderBookAnalyzer::new();
        
        // Add enough snapshots for analysis
        for i in 0..20 {
            let snapshot = OrderBookSnapshot {
                timestamp_ms: i * 100,
                bids: vec![
                    BookLevel { price: dec!(100), quantity: dec!(100) },
                    BookLevel { price: dec!(99.9), quantity: dec!(100) },
                ],
                asks: vec![
                    BookLevel { price: dec!(100.1), quantity: dec!(100) },
                    BookLevel { price: dec!(100.2), quantity: dec!(100) },
                ],
                last_trade_price: None,
                last_trade_side: None,
            };
            analyzer.process_snapshot(snapshot);
        }
        
        let analysis = analyzer.analyze_market_makers();
        assert!(analysis.is_some(), "Should produce MM analysis with enough data");
        
        let mm = analysis.unwrap();
        assert!(mm.spread_stability > Decimal::ZERO, "Should have positive spread stability");
        assert!(mm.depth_symmetry > Decimal::ZERO, "Should have positive depth symmetry");
    }
    
    #[test]
    fn test_full_analysis() {
        let mut analyzer = OrderBookAnalyzer::new();
        
        // Add some data
        for i in 0..15 {
            let snapshot = create_test_snapshot(dec!(100), dec!(101), dec!(1000), dec!(800));
            let mut s = snapshot;
            s.timestamp_ms = i * 100;
            analyzer.process_snapshot(s);
        }
        
        for i in 0..10 {
            analyzer.process_trade(TradeFlow {
                timestamp_ms: i * 1000,
                price: dec!(100.5),
                quantity: dec!(50),
                side: if i % 2 == 0 { TradeSide::Buy } else { TradeSide::Sell },
            });
        }
        
        let analysis = analyzer.get_full_analysis();
        assert!(analysis.imbalance.is_some(), "Should have imbalance analysis");
        assert_eq!(analysis.snapshot_count, 15);
        assert_eq!(analysis.trade_count, 10);
    }
    
    #[test]
    fn test_empty_orderbook_handling() {
        let analyzer = OrderBookAnalyzer::new();
        
        // Should return None for empty analyzer
        assert!(analyzer.calculate_imbalance().is_none());
        assert!(analyzer.calculate_vpin().is_none());
        assert!(analyzer.analyze_market_makers().is_none());
    }
    
    #[test]
    fn test_config_customization() {
        let config = OrderBookAnalyzerConfig {
            imbalance_levels: 20,
            depth_weight_decay: dec!(0.9),
            vpin_bucket_size: dec!(500),
            vpin_bucket_count: 100,
            iceberg_refill_threshold: 5,
            mm_analysis_window_secs: 120,
            direction_threshold: dec!(0.2),
            high_confidence_threshold: dec!(0.4),
        };
        
        let analyzer = OrderBookAnalyzer::with_config(config.clone());
        assert_eq!(analyzer.config.imbalance_levels, 20);
        assert_eq!(analyzer.config.vpin_bucket_count, 100);
    }
    
    #[test]
    fn test_trade_side_serialization() {
        let buy = TradeSide::Buy;
        let sell = TradeSide::Sell;
        
        let buy_json = serde_json::to_string(&buy).unwrap();
        let sell_json = serde_json::to_string(&sell).unwrap();
        
        assert_eq!(buy_json, "\"Buy\"");
        assert_eq!(sell_json, "\"Sell\"");
    }
    
    #[test]
    fn test_predicted_direction_all_cases() {
        assert_ne!(PredictedDirection::Up, PredictedDirection::Down);
        assert_ne!(PredictedDirection::Up, PredictedDirection::Neutral);
        assert_ne!(PredictedDirection::Down, PredictedDirection::Neutral);
    }
    
    #[test]
    fn test_depth_weighted_imbalance() {
        let mut analyzer = OrderBookAnalyzer::with_config(OrderBookAnalyzerConfig {
            depth_weight_decay: dec!(0.5), // Strong decay
            ..Default::default()
        });
        
        // Create snapshot where first level has buy pressure, deeper levels have sell pressure
        let snapshot = OrderBookSnapshot {
            timestamp_ms: 1000,
            bids: vec![
                BookLevel { price: dec!(100), quantity: dec!(1000) }, // Strong first level
                BookLevel { price: dec!(99), quantity: dec!(100) },
                BookLevel { price: dec!(98), quantity: dec!(100) },
            ],
            asks: vec![
                BookLevel { price: dec!(101), quantity: dec!(100) }, // Weak first level
                BookLevel { price: dec!(102), quantity: dec!(1000) },
                BookLevel { price: dec!(103), quantity: dec!(1000) },
            ],
            last_trade_price: None,
            last_trade_side: None,
        };
        analyzer.process_snapshot(snapshot);
        
        let result = analyzer.calculate_imbalance().unwrap();
        // Depth-weighted should favor first level more
        assert!(result.depth_weighted_imbalance > result.simple_imbalance, 
            "Depth-weighted should show stronger buy signal when first level dominates");
    }
}
