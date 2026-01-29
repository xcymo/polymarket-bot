//! Smart Order Routing (SOR) Module
//!
//! Professional-grade order routing system for optimal execution across multiple venues.
//!
//! # Features
//! - Multi-venue order splitting with cost optimization
//! - Real-time venue scoring (price, liquidity, fees, latency)
//! - Adaptive routing based on market conditions
//! - Child order management and aggregation
//! - Execution quality feedback loop
//!
//! # Example
//! ```ignore
//! use polymarket_bot::routing::{SmartOrderRouter, Venue, ParentOrder, RoutingConfig};
//!
//! let mut router = SmartOrderRouter::new(RoutingConfig::default());
//! router.register_venue(binance_venue);
//! router.register_venue(okx_venue);
//!
//! let parent = ParentOrder::new("BTC-USDT", Side::Buy, dec!(1.0), OrderType::Limit(dec!(100000)));
//! let child_orders = router.route(&parent)?;
//! ```

use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Order side
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Side {
    Buy,
    Sell,
}

/// Order type for routing
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OrderType {
    /// Market order - execute immediately at best available price
    Market,
    /// Limit order with specified price
    Limit(Decimal),
    /// Aggressive limit - limit but willing to cross spread
    AggressiveLimit(Decimal),
}

/// Venue status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VenueStatus {
    Active,
    Degraded,
    Unavailable,
}

/// Represents a trading venue (exchange, DEX, liquidity pool)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Venue {
    /// Unique venue identifier
    pub id: String,
    /// Human-readable name
    pub name: String,
    /// Current status
    pub status: VenueStatus,
    /// Maker fee (negative = rebate)
    pub maker_fee: Decimal,
    /// Taker fee
    pub taker_fee: Decimal,
    /// Minimum order size
    pub min_order_size: Decimal,
    /// Maximum order size (None = unlimited)
    pub max_order_size: Option<Decimal>,
    /// Typical latency in milliseconds
    pub latency_ms: u64,
    /// Supported symbols
    pub symbols: Vec<String>,
    /// Priority weight (higher = preferred)
    pub priority: u32,
}

impl Venue {
    /// Create a new venue
    pub fn new(id: &str, name: &str) -> Self {
        Self {
            id: id.to_string(),
            name: name.to_string(),
            status: VenueStatus::Active,
            maker_fee: dec!(0.001),
            taker_fee: dec!(0.001),
            min_order_size: dec!(0.0001),
            max_order_size: None,
            latency_ms: 50,
            symbols: Vec::new(),
            priority: 100,
        }
    }

    /// Set fees
    pub fn with_fees(mut self, maker: Decimal, taker: Decimal) -> Self {
        self.maker_fee = maker;
        self.taker_fee = taker;
        self
    }

    /// Set latency
    pub fn with_latency(mut self, latency_ms: u64) -> Self {
        self.latency_ms = latency_ms;
        self
    }

    /// Set order size limits
    pub fn with_size_limits(mut self, min: Decimal, max: Option<Decimal>) -> Self {
        self.min_order_size = min;
        self.max_order_size = max;
        self
    }

    /// Add supported symbol
    pub fn with_symbol(mut self, symbol: &str) -> Self {
        self.symbols.push(symbol.to_string());
        self
    }

    /// Check if venue supports a symbol
    pub fn supports_symbol(&self, symbol: &str) -> bool {
        self.symbols.is_empty() || self.symbols.iter().any(|s| s == symbol)
    }

    /// Check if venue is available for trading
    pub fn is_available(&self) -> bool {
        self.status == VenueStatus::Active || self.status == VenueStatus::Degraded
    }

    /// Get effective fee for order type
    pub fn effective_fee(&self, is_maker: bool) -> Decimal {
        if is_maker {
            self.maker_fee
        } else {
            self.taker_fee
        }
    }
}

/// Order book level (price + quantity)
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct BookLevel {
    pub price: Decimal,
    pub quantity: Decimal,
}

/// Venue liquidity snapshot
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VenueLiquidity {
    /// Venue ID
    pub venue_id: String,
    /// Symbol
    pub symbol: String,
    /// Best bid
    pub best_bid: Option<BookLevel>,
    /// Best ask
    pub best_ask: Option<BookLevel>,
    /// Bid depth (aggregated quantity at top N levels)
    pub bid_depth: Vec<BookLevel>,
    /// Ask depth (aggregated quantity at top N levels)
    pub ask_depth: Vec<BookLevel>,
    /// Timestamp
    pub timestamp: u64,
}

impl VenueLiquidity {
    /// Create new liquidity snapshot
    pub fn new(venue_id: &str, symbol: &str) -> Self {
        Self {
            venue_id: venue_id.to_string(),
            symbol: symbol.to_string(),
            best_bid: None,
            best_ask: None,
            bid_depth: Vec::new(),
            ask_depth: Vec::new(),
            timestamp: 0,
        }
    }

    /// Get mid price
    pub fn mid_price(&self) -> Option<Decimal> {
        match (self.best_bid, self.best_ask) {
            (Some(bid), Some(ask)) => Some((bid.price + ask.price) / dec!(2)),
            _ => None,
        }
    }

    /// Get spread in basis points
    pub fn spread_bps(&self) -> Option<Decimal> {
        match (self.best_bid, self.best_ask) {
            (Some(bid), Some(ask)) if bid.price > Decimal::ZERO => {
                let spread = (ask.price - bid.price) / bid.price * dec!(10000);
                Some(spread)
            }
            _ => None,
        }
    }

    /// Get available quantity at or better than price
    pub fn available_quantity(&self, side: Side, limit_price: Option<Decimal>) -> Decimal {
        let levels = match side {
            Side::Buy => &self.ask_depth,
            Side::Sell => &self.bid_depth,
        };

        levels
            .iter()
            .filter(|level| {
                if let Some(limit) = limit_price {
                    match side {
                        Side::Buy => level.price <= limit,
                        Side::Sell => level.price >= limit,
                    }
                } else {
                    true
                }
            })
            .map(|level| level.quantity)
            .sum()
    }

    /// Estimate average execution price for given quantity
    pub fn estimate_avg_price(&self, side: Side, quantity: Decimal) -> Option<Decimal> {
        let levels = match side {
            Side::Buy => &self.ask_depth,
            Side::Sell => &self.bid_depth,
        };

        if levels.is_empty() {
            return None;
        }

        let mut remaining = quantity;
        let mut total_cost = Decimal::ZERO;

        for level in levels {
            if remaining <= Decimal::ZERO {
                break;
            }
            let fill_qty = remaining.min(level.quantity);
            total_cost += fill_qty * level.price;
            remaining -= fill_qty;
        }

        if remaining > Decimal::ZERO {
            // Not enough liquidity
            return None;
        }

        Some(total_cost / quantity)
    }

    /// Estimate price impact for given quantity (in basis points)
    pub fn estimate_impact_bps(&self, side: Side, quantity: Decimal) -> Option<Decimal> {
        let mid = self.mid_price()?;
        let avg_price = self.estimate_avg_price(side, quantity)?;

        let impact = match side {
            Side::Buy => (avg_price - mid) / mid * dec!(10000),
            Side::Sell => (mid - avg_price) / mid * dec!(10000),
        };

        Some(impact)
    }
}

/// Parent order to be routed
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParentOrder {
    /// Unique order ID
    pub id: String,
    /// Symbol
    pub symbol: String,
    /// Side
    pub side: Side,
    /// Total quantity to execute
    pub quantity: Decimal,
    /// Order type
    pub order_type: OrderType,
    /// Maximum venues to use (None = unlimited)
    pub max_venues: Option<usize>,
    /// Urgency level (0.0 = patient, 1.0 = immediate)
    pub urgency: f64,
    /// Excluded venues
    pub excluded_venues: Vec<String>,
    /// Created timestamp
    pub created_at: u64,
}

impl ParentOrder {
    /// Create a new parent order
    pub fn new(symbol: &str, side: Side, quantity: Decimal, order_type: OrderType) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            symbol: symbol.to_string(),
            side,
            quantity,
            order_type,
            max_venues: None,
            urgency: 0.5,
            excluded_venues: Vec::new(),
            created_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
        }
    }

    /// Set maximum venues
    pub fn with_max_venues(mut self, max: usize) -> Self {
        self.max_venues = Some(max);
        self
    }

    /// Set urgency
    pub fn with_urgency(mut self, urgency: f64) -> Self {
        self.urgency = urgency.clamp(0.0, 1.0);
        self
    }

    /// Exclude a venue
    pub fn exclude_venue(mut self, venue_id: &str) -> Self {
        self.excluded_venues.push(venue_id.to_string());
        self
    }

    /// Get limit price if applicable
    pub fn limit_price(&self) -> Option<Decimal> {
        match self.order_type {
            OrderType::Limit(price) | OrderType::AggressiveLimit(price) => Some(price),
            OrderType::Market => None,
        }
    }
}

/// Child order generated by router
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChildOrder {
    /// Unique child order ID
    pub id: String,
    /// Parent order ID
    pub parent_id: String,
    /// Target venue ID
    pub venue_id: String,
    /// Symbol
    pub symbol: String,
    /// Side
    pub side: Side,
    /// Quantity for this child
    pub quantity: Decimal,
    /// Execution price (limit or expected)
    pub price: Decimal,
    /// Whether this is a maker order
    pub is_maker: bool,
    /// Expected fee
    pub expected_fee: Decimal,
    /// Expected slippage in bps
    pub expected_slippage_bps: Decimal,
    /// Sequence number (for ordering)
    pub sequence: u32,
    /// Created timestamp
    pub created_at: u64,
}

impl ChildOrder {
    /// Create a new child order
    pub fn new(
        parent_id: &str,
        venue_id: &str,
        symbol: &str,
        side: Side,
        quantity: Decimal,
        price: Decimal,
    ) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            parent_id: parent_id.to_string(),
            venue_id: venue_id.to_string(),
            symbol: symbol.to_string(),
            side,
            quantity,
            price,
            is_maker: false,
            expected_fee: Decimal::ZERO,
            expected_slippage_bps: Decimal::ZERO,
            sequence: 0,
            created_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
        }
    }

    /// Calculate notional value
    pub fn notional(&self) -> Decimal {
        self.quantity * self.price
    }

    /// Calculate total cost including fees
    pub fn total_cost(&self) -> Decimal {
        let notional = self.notional();
        let fee_cost = notional * self.expected_fee;
        match self.side {
            Side::Buy => notional + fee_cost,
            Side::Sell => notional - fee_cost,
        }
    }
}

/// Venue score components
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VenueScore {
    /// Venue ID
    pub venue_id: String,
    /// Price score (0-100, higher = better price)
    pub price_score: f64,
    /// Liquidity score (0-100, higher = more liquidity)
    pub liquidity_score: f64,
    /// Fee score (0-100, higher = lower fees)
    pub fee_score: f64,
    /// Latency score (0-100, higher = lower latency)
    pub latency_score: f64,
    /// Reliability score (0-100, based on recent performance)
    pub reliability_score: f64,
    /// Combined weighted score
    pub total_score: f64,
    /// Maximum fillable quantity
    pub max_fill_quantity: Decimal,
    /// Expected average price
    pub expected_price: Option<Decimal>,
}

impl VenueScore {
    /// Create a new venue score
    pub fn new(venue_id: &str) -> Self {
        Self {
            venue_id: venue_id.to_string(),
            price_score: 50.0,
            liquidity_score: 50.0,
            fee_score: 50.0,
            latency_score: 50.0,
            reliability_score: 100.0,
            total_score: 50.0,
            max_fill_quantity: Decimal::ZERO,
            expected_price: None,
        }
    }

    /// Calculate weighted total score
    pub fn calculate_total(&mut self, weights: &ScoreWeights) {
        self.total_score = self.price_score * weights.price
            + self.liquidity_score * weights.liquidity
            + self.fee_score * weights.fee
            + self.latency_score * weights.latency
            + self.reliability_score * weights.reliability;
    }
}

/// Weights for venue scoring
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct ScoreWeights {
    pub price: f64,
    pub liquidity: f64,
    pub fee: f64,
    pub latency: f64,
    pub reliability: f64,
}

impl Default for ScoreWeights {
    fn default() -> Self {
        Self {
            price: 0.35,
            liquidity: 0.25,
            fee: 0.15,
            latency: 0.15,
            reliability: 0.10,
        }
    }
}

impl ScoreWeights {
    /// Weights optimized for urgent orders
    pub fn urgent() -> Self {
        Self {
            price: 0.20,
            liquidity: 0.35,
            fee: 0.10,
            latency: 0.25,
            reliability: 0.10,
        }
    }

    /// Weights optimized for patient orders
    pub fn patient() -> Self {
        Self {
            price: 0.45,
            liquidity: 0.15,
            fee: 0.25,
            latency: 0.05,
            reliability: 0.10,
        }
    }

    /// Interpolate weights based on urgency
    pub fn for_urgency(urgency: f64) -> Self {
        let patient = Self::patient();
        let urgent = Self::urgent();
        let u = urgency.clamp(0.0, 1.0);

        Self {
            price: patient.price * (1.0 - u) + urgent.price * u,
            liquidity: patient.liquidity * (1.0 - u) + urgent.liquidity * u,
            fee: patient.fee * (1.0 - u) + urgent.fee * u,
            latency: patient.latency * (1.0 - u) + urgent.latency * u,
            reliability: patient.reliability * (1.0 - u) + urgent.reliability * u,
        }
    }
}

/// Routing algorithm
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RoutingAlgorithm {
    /// Route to single best venue
    BestVenue,
    /// Split across venues proportionally to liquidity
    ProRata,
    /// Optimize for minimum cost (price + fees + impact)
    MinCost,
    /// Minimize market impact
    MinImpact,
    /// Spray across all venues equally
    Spray,
}

/// Routing configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingConfig {
    /// Routing algorithm
    pub algorithm: RoutingAlgorithm,
    /// Score weights
    pub weights: ScoreWeights,
    /// Minimum child order size (as fraction of total)
    pub min_child_fraction: Decimal,
    /// Maximum venues to use
    pub max_venues: usize,
    /// Whether to allow partial fills
    pub allow_partial: bool,
    /// Maximum acceptable slippage in bps
    pub max_slippage_bps: Decimal,
    /// Retry on venue failure
    pub retry_on_failure: bool,
    /// Stale data threshold (ms)
    pub stale_threshold_ms: u64,
}

impl Default for RoutingConfig {
    fn default() -> Self {
        Self {
            algorithm: RoutingAlgorithm::MinCost,
            weights: ScoreWeights::default(),
            min_child_fraction: dec!(0.05),
            max_venues: 5,
            allow_partial: true,
            max_slippage_bps: dec!(50),
            retry_on_failure: true,
            stale_threshold_ms: 5000,
        }
    }
}

/// Routing decision result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingDecision {
    /// Parent order ID
    pub parent_id: String,
    /// Generated child orders
    pub child_orders: Vec<ChildOrder>,
    /// Venue scores used
    pub venue_scores: Vec<VenueScore>,
    /// Algorithm used
    pub algorithm: RoutingAlgorithm,
    /// Expected total cost
    pub expected_total_cost: Decimal,
    /// Expected average price
    pub expected_avg_price: Decimal,
    /// Expected total slippage in bps
    pub expected_slippage_bps: Decimal,
    /// Coverage (filled / requested)
    pub coverage: Decimal,
    /// Decision timestamp
    pub timestamp: u64,
    /// Computation time in microseconds
    pub compute_time_us: u64,
}

impl RoutingDecision {
    /// Check if order can be fully filled
    pub fn is_fully_covered(&self) -> bool {
        self.coverage >= dec!(0.9999)
    }

    /// Get number of venues used
    pub fn num_venues(&self) -> usize {
        self.child_orders
            .iter()
            .map(|c| &c.venue_id)
            .collect::<std::collections::HashSet<_>>()
            .len()
    }
}

/// Execution feedback for learning
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionFeedback {
    /// Child order ID
    pub child_id: String,
    /// Venue ID
    pub venue_id: String,
    /// Requested quantity
    pub requested_qty: Decimal,
    /// Filled quantity
    pub filled_qty: Decimal,
    /// Requested price
    pub requested_price: Decimal,
    /// Actual average fill price
    pub actual_price: Decimal,
    /// Actual slippage in bps
    pub actual_slippage_bps: Decimal,
    /// Execution latency in ms
    pub latency_ms: u64,
    /// Whether execution succeeded
    pub success: bool,
    /// Error message if failed
    pub error: Option<String>,
}

/// Venue performance metrics (for adaptive scoring)
#[derive(Debug, Clone, Default)]
pub struct VenueMetrics {
    /// Total orders sent
    pub total_orders: u64,
    /// Successful fills
    pub successful_fills: u64,
    /// Partial fills
    pub partial_fills: u64,
    /// Failed orders
    pub failed_orders: u64,
    /// Average slippage (bps)
    pub avg_slippage_bps: f64,
    /// Average latency (ms)
    pub avg_latency_ms: f64,
    /// Recent reliability (0-1)
    pub recent_reliability: f64,
    /// Last updated
    pub last_updated: Option<Instant>,
}

impl VenueMetrics {
    /// Update metrics with execution feedback
    pub fn update(&mut self, feedback: &ExecutionFeedback) {
        self.total_orders += 1;

        if feedback.success {
            if feedback.filled_qty >= feedback.requested_qty * dec!(0.99) {
                self.successful_fills += 1;
            } else if feedback.filled_qty > Decimal::ZERO {
                self.partial_fills += 1;
            }
        } else {
            self.failed_orders += 1;
        }

        // Update running averages
        let n = self.total_orders as f64;
        let slippage_f64 = feedback
            .actual_slippage_bps
            .to_string()
            .parse::<f64>()
            .unwrap_or(0.0);
        self.avg_slippage_bps =
            self.avg_slippage_bps * (n - 1.0) / n + slippage_f64 / n;
        self.avg_latency_ms =
            self.avg_latency_ms * (n - 1.0) / n + (feedback.latency_ms as f64) / n;

        // Calculate recent reliability (exponential decay)
        let success_weight = if feedback.success { 1.0 } else { 0.0 };
        self.recent_reliability = self.recent_reliability * 0.95 + success_weight * 0.05;

        self.last_updated = Some(Instant::now());
    }

    /// Get fill rate
    pub fn fill_rate(&self) -> f64 {
        if self.total_orders == 0 {
            1.0
        } else {
            (self.successful_fills + self.partial_fills) as f64 / self.total_orders as f64
        }
    }
}

/// Smart Order Router
pub struct SmartOrderRouter {
    /// Configuration
    config: RoutingConfig,
    /// Registered venues
    venues: HashMap<String, Venue>,
    /// Venue liquidity snapshots
    liquidity: HashMap<String, VenueLiquidity>,
    /// Venue performance metrics
    metrics: HashMap<String, VenueMetrics>,
    /// Order counter for sequencing
    order_counter: u32,
}

impl SmartOrderRouter {
    /// Create a new router
    pub fn new(config: RoutingConfig) -> Self {
        Self {
            config,
            venues: HashMap::new(),
            liquidity: HashMap::new(),
            metrics: HashMap::new(),
            order_counter: 0,
        }
    }

    /// Register a venue
    pub fn register_venue(&mut self, venue: Venue) {
        let venue_id = venue.id.clone();
        self.venues.insert(venue_id.clone(), venue);
        self.metrics.insert(venue_id, VenueMetrics::default());
    }

    /// Remove a venue
    pub fn remove_venue(&mut self, venue_id: &str) {
        self.venues.remove(venue_id);
        self.liquidity.remove(venue_id);
        self.metrics.remove(venue_id);
    }

    /// Update venue liquidity
    pub fn update_liquidity(&mut self, liquidity: VenueLiquidity) {
        let key = format!("{}:{}", liquidity.venue_id, liquidity.symbol);
        self.liquidity.insert(key, liquidity);
    }

    /// Update venue status
    pub fn update_venue_status(&mut self, venue_id: &str, status: VenueStatus) {
        if let Some(venue) = self.venues.get_mut(venue_id) {
            venue.status = status;
        }
    }

    /// Record execution feedback
    pub fn record_feedback(&mut self, feedback: ExecutionFeedback) {
        if let Some(metrics) = self.metrics.get_mut(&feedback.venue_id) {
            metrics.update(&feedback);
        }
    }

    /// Get available venues for a symbol
    fn get_available_venues(&self, symbol: &str, excluded: &[String]) -> Vec<&Venue> {
        self.venues
            .values()
            .filter(|v| {
                v.is_available()
                    && v.supports_symbol(symbol)
                    && !excluded.contains(&v.id)
            })
            .collect()
    }

    /// Score venues for an order
    fn score_venues(
        &self,
        order: &ParentOrder,
        venues: &[&Venue],
        weights: &ScoreWeights,
    ) -> Vec<VenueScore> {
        let mut scores: Vec<VenueScore> = Vec::new();

        // Collect reference prices for normalization
        let mut all_prices: Vec<Decimal> = Vec::new();
        let mut all_fees: Vec<Decimal> = Vec::new();
        let mut all_latencies: Vec<u64> = Vec::new();

        for venue in venues {
            let key = format!("{}:{}", venue.id, order.symbol);
            if let Some(liq) = self.liquidity.get(&key) {
                if let Some(price) = liq.estimate_avg_price(order.side, order.quantity) {
                    all_prices.push(price);
                }
            }
            all_fees.push(venue.taker_fee);
            all_latencies.push(venue.latency_ms);
        }

        // Calculate min/max for normalization
        let (min_price, max_price) = if all_prices.is_empty() {
            (dec!(0), dec!(1))
        } else {
            let min = *all_prices.iter().min().unwrap();
            let max = *all_prices.iter().max().unwrap();
            if min == max {
                (min, min + dec!(1))
            } else {
                (min, max)
            }
        };

        let min_fee = *all_fees.iter().min().unwrap_or(&dec!(0));
        let max_fee = *all_fees.iter().max().unwrap_or(&dec!(0.01));
        let fee_range = if max_fee == min_fee {
            dec!(0.01)
        } else {
            max_fee - min_fee
        };

        let min_latency = *all_latencies.iter().min().unwrap_or(&1) as f64;
        let max_latency = *all_latencies.iter().max().unwrap_or(&100) as f64;
        let latency_range = if (max_latency - min_latency).abs() < 1.0 {
            100.0
        } else {
            max_latency - min_latency
        };

        for venue in venues {
            let mut score = VenueScore::new(&venue.id);
            let key = format!("{}:{}", venue.id, order.symbol);

            // Price score
            if let Some(liq) = self.liquidity.get(&key) {
                if let Some(avg_price) = liq.estimate_avg_price(order.side, order.quantity) {
                    score.expected_price = Some(avg_price);
                    let price_range = max_price - min_price;
                    if price_range > Decimal::ZERO {
                        // For buys, lower price is better; for sells, higher is better
                        let normalized = match order.side {
                            Side::Buy => {
                                let diff = max_price - avg_price;
                                diff / price_range
                            }
                            Side::Sell => {
                                let diff = avg_price - min_price;
                                diff / price_range
                            }
                        };
                        let norm_f64: f64 = normalized.to_string().parse().unwrap_or(0.5);
                        score.price_score = norm_f64 * 100.0;
                    }
                }

                // Liquidity score
                let available = liq.available_quantity(order.side, order.limit_price());
                let fill_ratio = if order.quantity > Decimal::ZERO {
                    (available / order.quantity).min(dec!(1))
                } else {
                    dec!(0)
                };
                let fill_f64: f64 = fill_ratio.to_string().parse().unwrap_or(0.0);
                score.liquidity_score = fill_f64 * 100.0;
                score.max_fill_quantity = available.min(order.quantity);
            }

            // Fee score (lower fee = higher score)
            let fee_normalized = (max_fee - venue.taker_fee) / fee_range;
            let fee_f64: f64 = fee_normalized.to_string().parse().unwrap_or(0.5);
            score.fee_score = fee_f64.clamp(0.0, 1.0) * 100.0;

            // Latency score (lower latency = higher score)
            let latency_normalized =
                (max_latency - venue.latency_ms as f64) / latency_range;
            score.latency_score = latency_normalized.clamp(0.0, 1.0) * 100.0;

            // Reliability score from metrics
            if let Some(metrics) = self.metrics.get(&venue.id) {
                score.reliability_score = metrics.recent_reliability * 100.0;
            }

            score.calculate_total(weights);
            scores.push(score);
        }

        // Sort by total score descending
        scores.sort_by(|a, b| b.total_score.partial_cmp(&a.total_score).unwrap());
        scores
    }

    /// Route order to best single venue
    fn route_best_venue(
        &mut self,
        order: &ParentOrder,
        scores: &[VenueScore],
    ) -> Vec<ChildOrder> {
        if scores.is_empty() {
            return Vec::new();
        }

        let best = &scores[0];
        if best.max_fill_quantity <= Decimal::ZERO {
            return Vec::new();
        }

        let venue = match self.venues.get(&best.venue_id) {
            Some(v) => v,
            None => return Vec::new(),
        };

        let qty = best.max_fill_quantity.min(order.quantity);
        let price = best.expected_price.unwrap_or(dec!(0));

        self.order_counter += 1;
        let mut child = ChildOrder::new(
            &order.id,
            &best.venue_id,
            &order.symbol,
            order.side,
            qty,
            price,
        );
        child.sequence = self.order_counter;
        child.expected_fee = venue.taker_fee;

        vec![child]
    }

    /// Route order proportionally to liquidity
    fn route_pro_rata(
        &mut self,
        order: &ParentOrder,
        scores: &[VenueScore],
    ) -> Vec<ChildOrder> {
        let mut children = Vec::new();
        let min_qty = order.quantity * self.config.min_child_fraction;

        // Calculate total available liquidity
        let total_liquidity: Decimal = scores.iter().map(|s| s.max_fill_quantity).sum();

        if total_liquidity <= Decimal::ZERO {
            return children;
        }

        let mut remaining = order.quantity;
        let max_venues = self.config.max_venues.min(scores.len());

        for score in scores.iter().take(max_venues) {
            if remaining <= Decimal::ZERO {
                break;
            }

            let venue = match self.venues.get(&score.venue_id) {
                Some(v) => v,
                None => continue,
            };

            // Pro-rata allocation
            let proportion = score.max_fill_quantity / total_liquidity;
            let target_qty = (order.quantity * proportion).min(score.max_fill_quantity);
            let qty = target_qty.min(remaining);

            if qty < min_qty {
                continue;
            }

            let price = score.expected_price.unwrap_or(dec!(0));

            self.order_counter += 1;
            let mut child = ChildOrder::new(
                &order.id,
                &score.venue_id,
                &order.symbol,
                order.side,
                qty,
                price,
            );
            child.sequence = self.order_counter;
            child.expected_fee = venue.taker_fee;

            remaining -= qty;
            children.push(child);
        }

        children
    }

    /// Route order optimizing for minimum total cost
    fn route_min_cost(
        &mut self,
        order: &ParentOrder,
        scores: &[VenueScore],
    ) -> Vec<ChildOrder> {
        let mut children = Vec::new();
        let min_qty = order.quantity * self.config.min_child_fraction;
        let mut remaining = order.quantity;
        let max_venues = self.config.max_venues.min(scores.len());

        // Greedy: fill from lowest cost venue first
        // Sort by effective cost (price + fees)
        let mut cost_sorted: Vec<_> = scores
            .iter()
            .filter_map(|s| {
                let venue = self.venues.get(&s.venue_id)?;
                let price = s.expected_price?;
                let fee_adj = match order.side {
                    Side::Buy => price * (dec!(1) + venue.taker_fee),
                    Side::Sell => price * (dec!(1) - venue.taker_fee),
                };
                Some((s, fee_adj))
            })
            .collect();

        cost_sorted.sort_by(|a, b| {
            match order.side {
                Side::Buy => a.1.cmp(&b.1),   // Lower cost better for buying
                Side::Sell => b.1.cmp(&a.1),  // Higher proceeds better for selling
            }
        });

        for (score, _cost) in cost_sorted.into_iter().take(max_venues) {
            if remaining <= Decimal::ZERO {
                break;
            }

            let venue = match self.venues.get(&score.venue_id) {
                Some(v) => v,
                None => continue,
            };

            let qty = score.max_fill_quantity.min(remaining);
            if qty < min_qty {
                continue;
            }

            let price = score.expected_price.unwrap_or(dec!(0));

            self.order_counter += 1;
            let mut child = ChildOrder::new(
                &order.id,
                &score.venue_id,
                &order.symbol,
                order.side,
                qty,
                price,
            );
            child.sequence = self.order_counter;
            child.expected_fee = venue.taker_fee;

            remaining -= qty;
            children.push(child);
        }

        children
    }

    /// Route order minimizing market impact
    fn route_min_impact(
        &mut self,
        order: &ParentOrder,
        scores: &[VenueScore],
    ) -> Vec<ChildOrder> {
        let mut children = Vec::new();
        let min_qty = order.quantity * self.config.min_child_fraction;
        let mut remaining = order.quantity;
        let max_venues = self.config.max_venues.min(scores.len());

        // Sort by liquidity score (more liquidity = less impact)
        let mut liq_sorted: Vec<_> = scores.iter().collect();
        liq_sorted.sort_by(|a, b| {
            b.liquidity_score
                .partial_cmp(&a.liquidity_score)
                .unwrap()
        });

        // Spread across venues to minimize individual impact
        let venue_count = liq_sorted
            .iter()
            .filter(|s| s.max_fill_quantity >= min_qty)
            .count()
            .min(max_venues);

        if venue_count == 0 {
            return children;
        }

        let base_allocation = order.quantity / Decimal::from(venue_count as u32);

        for score in liq_sorted.into_iter().take(max_venues) {
            if remaining <= Decimal::ZERO {
                break;
            }

            let venue = match self.venues.get(&score.venue_id) {
                Some(v) => v,
                None => continue,
            };

            // Allocate base amount, capped by available liquidity
            let qty = base_allocation
                .min(score.max_fill_quantity)
                .min(remaining);

            if qty < min_qty {
                continue;
            }

            let price = score.expected_price.unwrap_or(dec!(0));

            self.order_counter += 1;
            let mut child = ChildOrder::new(
                &order.id,
                &score.venue_id,
                &order.symbol,
                order.side,
                qty,
                price,
            );
            child.sequence = self.order_counter;
            child.expected_fee = venue.taker_fee;

            remaining -= qty;
            children.push(child);
        }

        children
    }

    /// Route order equally across all venues (spray)
    fn route_spray(
        &mut self,
        order: &ParentOrder,
        scores: &[VenueScore],
    ) -> Vec<ChildOrder> {
        let mut children = Vec::new();
        let min_qty = order.quantity * self.config.min_child_fraction;
        let max_venues = self.config.max_venues.min(scores.len());

        let eligible: Vec<_> = scores
            .iter()
            .filter(|s| s.max_fill_quantity >= min_qty)
            .take(max_venues)
            .collect();

        if eligible.is_empty() {
            return children;
        }

        let equal_qty = order.quantity / Decimal::from(eligible.len() as u32);
        let mut remaining = order.quantity;

        for score in eligible {
            if remaining <= Decimal::ZERO {
                break;
            }

            let venue = match self.venues.get(&score.venue_id) {
                Some(v) => v,
                None => continue,
            };

            let qty = equal_qty.min(score.max_fill_quantity).min(remaining);
            let price = score.expected_price.unwrap_or(dec!(0));

            self.order_counter += 1;
            let mut child = ChildOrder::new(
                &order.id,
                &score.venue_id,
                &order.symbol,
                order.side,
                qty,
                price,
            );
            child.sequence = self.order_counter;
            child.expected_fee = venue.taker_fee;

            remaining -= qty;
            children.push(child);
        }

        children
    }

    /// Main routing function
    pub fn route(&mut self, order: &ParentOrder) -> RoutingDecision {
        let start = Instant::now();

        // Get available venues
        let venues = self.get_available_venues(&order.symbol, &order.excluded_venues);

        // Adjust weights based on urgency
        let weights = ScoreWeights::for_urgency(order.urgency);

        // Score venues
        let scores = self.score_venues(order, &venues, &weights);

        // Apply routing algorithm
        let child_orders = match self.config.algorithm {
            RoutingAlgorithm::BestVenue => self.route_best_venue(order, &scores),
            RoutingAlgorithm::ProRata => self.route_pro_rata(order, &scores),
            RoutingAlgorithm::MinCost => self.route_min_cost(order, &scores),
            RoutingAlgorithm::MinImpact => self.route_min_impact(order, &scores),
            RoutingAlgorithm::Spray => self.route_spray(order, &scores),
        };

        // Respect max_venues from parent order
        let child_orders = if let Some(max) = order.max_venues {
            child_orders.into_iter().take(max).collect()
        } else {
            child_orders
        };

        // Calculate aggregates
        let total_qty: Decimal = child_orders.iter().map(|c| c.quantity).sum();
        let coverage = if order.quantity > Decimal::ZERO {
            total_qty / order.quantity
        } else {
            dec!(0)
        };

        let total_cost: Decimal = child_orders
            .iter()
            .map(|c| c.quantity * c.price * (dec!(1) + c.expected_fee))
            .sum();

        let expected_avg_price = if total_qty > Decimal::ZERO {
            child_orders
                .iter()
                .map(|c| c.quantity * c.price)
                .sum::<Decimal>()
                / total_qty
        } else {
            dec!(0)
        };

        // Estimate slippage vs mid
        let mid_price = scores
            .iter()
            .filter_map(|s| s.expected_price)
            .next()
            .unwrap_or(expected_avg_price);

        let slippage_bps = if mid_price > Decimal::ZERO {
            match order.side {
                Side::Buy => (expected_avg_price - mid_price) / mid_price * dec!(10000),
                Side::Sell => (mid_price - expected_avg_price) / mid_price * dec!(10000),
            }
        } else {
            dec!(0)
        };

        let compute_time = start.elapsed();

        RoutingDecision {
            parent_id: order.id.clone(),
            child_orders,
            venue_scores: scores,
            algorithm: self.config.algorithm,
            expected_total_cost: total_cost,
            expected_avg_price,
            expected_slippage_bps: slippage_bps,
            coverage,
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
            compute_time_us: compute_time.as_micros() as u64,
        }
    }

    /// Get current venue count
    pub fn venue_count(&self) -> usize {
        self.venues.len()
    }

    /// Get venue by ID
    pub fn get_venue(&self, venue_id: &str) -> Option<&Venue> {
        self.venues.get(venue_id)
    }

    /// Get venue metrics
    pub fn get_metrics(&self, venue_id: &str) -> Option<&VenueMetrics> {
        self.metrics.get(venue_id)
    }

    /// Get configuration
    pub fn config(&self) -> &RoutingConfig {
        &self.config
    }

    /// Update configuration
    pub fn set_config(&mut self, config: RoutingConfig) {
        self.config = config;
    }
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_test_router() -> SmartOrderRouter {
        let config = RoutingConfig::default();
        let mut router = SmartOrderRouter::new(config);

        // Register test venues
        let binance = Venue::new("binance", "Binance")
            .with_fees(dec!(0.0002), dec!(0.0004))
            .with_latency(20)
            .with_symbol("BTC-USDT");

        let okx = Venue::new("okx", "OKX")
            .with_fees(dec!(0.0002), dec!(0.0005))
            .with_latency(30)
            .with_symbol("BTC-USDT");

        let bybit = Venue::new("bybit", "Bybit")
            .with_fees(dec!(0.0001), dec!(0.0006))
            .with_latency(25)
            .with_symbol("BTC-USDT");

        router.register_venue(binance);
        router.register_venue(okx);
        router.register_venue(bybit);

        // Add liquidity snapshots
        let mut binance_liq = VenueLiquidity::new("binance", "BTC-USDT");
        binance_liq.best_bid = Some(BookLevel {
            price: dec!(99990),
            quantity: dec!(10),
        });
        binance_liq.best_ask = Some(BookLevel {
            price: dec!(100010),
            quantity: dec!(10),
        });
        binance_liq.ask_depth = vec![
            BookLevel { price: dec!(100010), quantity: dec!(5) },
            BookLevel { price: dec!(100020), quantity: dec!(10) },
            BookLevel { price: dec!(100050), quantity: dec!(20) },
        ];
        binance_liq.bid_depth = vec![
            BookLevel { price: dec!(99990), quantity: dec!(5) },
            BookLevel { price: dec!(99980), quantity: dec!(10) },
            BookLevel { price: dec!(99950), quantity: dec!(20) },
        ];

        let mut okx_liq = VenueLiquidity::new("okx", "BTC-USDT");
        okx_liq.best_bid = Some(BookLevel {
            price: dec!(99985),
            quantity: dec!(8),
        });
        okx_liq.best_ask = Some(BookLevel {
            price: dec!(100015),
            quantity: dec!(8),
        });
        okx_liq.ask_depth = vec![
            BookLevel { price: dec!(100015), quantity: dec!(4) },
            BookLevel { price: dec!(100025), quantity: dec!(8) },
            BookLevel { price: dec!(100060), quantity: dec!(15) },
        ];
        okx_liq.bid_depth = vec![
            BookLevel { price: dec!(99985), quantity: dec!(4) },
            BookLevel { price: dec!(99975), quantity: dec!(8) },
            BookLevel { price: dec!(99940), quantity: dec!(15) },
        ];

        let mut bybit_liq = VenueLiquidity::new("bybit", "BTC-USDT");
        bybit_liq.best_bid = Some(BookLevel {
            price: dec!(99988),
            quantity: dec!(12),
        });
        bybit_liq.best_ask = Some(BookLevel {
            price: dec!(100012),
            quantity: dec!(12),
        });
        bybit_liq.ask_depth = vec![
            BookLevel { price: dec!(100012), quantity: dec!(6) },
            BookLevel { price: dec!(100022), quantity: dec!(12) },
            BookLevel { price: dec!(100055), quantity: dec!(25) },
        ];
        bybit_liq.bid_depth = vec![
            BookLevel { price: dec!(99988), quantity: dec!(6) },
            BookLevel { price: dec!(99978), quantity: dec!(12) },
            BookLevel { price: dec!(99945), quantity: dec!(25) },
        ];

        router.update_liquidity(binance_liq);
        router.update_liquidity(okx_liq);
        router.update_liquidity(bybit_liq);

        router
    }

    #[test]
    fn test_venue_creation() {
        let venue = Venue::new("test", "Test Exchange")
            .with_fees(dec!(0.001), dec!(0.002))
            .with_latency(50)
            .with_size_limits(dec!(0.001), Some(dec!(100)));

        assert_eq!(venue.id, "test");
        assert_eq!(venue.maker_fee, dec!(0.001));
        assert_eq!(venue.taker_fee, dec!(0.002));
        assert_eq!(venue.latency_ms, 50);
        assert_eq!(venue.min_order_size, dec!(0.001));
        assert_eq!(venue.max_order_size, Some(dec!(100)));
    }

    #[test]
    fn test_venue_supports_symbol() {
        let venue = Venue::new("test", "Test")
            .with_symbol("BTC-USDT")
            .with_symbol("ETH-USDT");

        assert!(venue.supports_symbol("BTC-USDT"));
        assert!(venue.supports_symbol("ETH-USDT"));
        assert!(!venue.supports_symbol("SOL-USDT"));

        // Empty symbols means supports all
        let venue_all = Venue::new("all", "All Symbols");
        assert!(venue_all.supports_symbol("ANY-PAIR"));
    }

    #[test]
    fn test_venue_is_available() {
        let mut venue = Venue::new("test", "Test");
        assert!(venue.is_available());

        venue.status = VenueStatus::Degraded;
        assert!(venue.is_available());

        venue.status = VenueStatus::Unavailable;
        assert!(!venue.is_available());
    }

    #[test]
    fn test_liquidity_mid_price() {
        let mut liq = VenueLiquidity::new("test", "BTC-USDT");
        assert!(liq.mid_price().is_none());

        liq.best_bid = Some(BookLevel {
            price: dec!(100),
            quantity: dec!(1),
        });
        liq.best_ask = Some(BookLevel {
            price: dec!(102),
            quantity: dec!(1),
        });

        assert_eq!(liq.mid_price(), Some(dec!(101)));
    }

    #[test]
    fn test_liquidity_spread_bps() {
        let mut liq = VenueLiquidity::new("test", "BTC-USDT");
        liq.best_bid = Some(BookLevel {
            price: dec!(10000),
            quantity: dec!(1),
        });
        liq.best_ask = Some(BookLevel {
            price: dec!(10010),
            quantity: dec!(1),
        });

        let spread = liq.spread_bps().unwrap();
        assert_eq!(spread, dec!(10)); // 10 bps
    }

    #[test]
    fn test_liquidity_available_quantity() {
        let mut liq = VenueLiquidity::new("test", "BTC-USDT");
        liq.ask_depth = vec![
            BookLevel { price: dec!(100), quantity: dec!(5) },
            BookLevel { price: dec!(101), quantity: dec!(10) },
            BookLevel { price: dec!(102), quantity: dec!(20) },
        ];

        // All available for market buy
        assert_eq!(liq.available_quantity(Side::Buy, None), dec!(35));

        // Limited by price
        assert_eq!(liq.available_quantity(Side::Buy, Some(dec!(101))), dec!(15));
        assert_eq!(liq.available_quantity(Side::Buy, Some(dec!(100))), dec!(5));
    }

    #[test]
    fn test_liquidity_estimate_avg_price() {
        let mut liq = VenueLiquidity::new("test", "BTC-USDT");
        liq.ask_depth = vec![
            BookLevel { price: dec!(100), quantity: dec!(5) },
            BookLevel { price: dec!(110), quantity: dec!(5) },
        ];

        // Buy 5 at 100
        let avg = liq.estimate_avg_price(Side::Buy, dec!(5)).unwrap();
        assert_eq!(avg, dec!(100));

        // Buy 10: 5 at 100 + 5 at 110 = avg 105
        let avg = liq.estimate_avg_price(Side::Buy, dec!(10)).unwrap();
        assert_eq!(avg, dec!(105));

        // Buy more than available
        assert!(liq.estimate_avg_price(Side::Buy, dec!(20)).is_none());
    }

    #[test]
    fn test_liquidity_estimate_impact() {
        let mut liq = VenueLiquidity::new("test", "BTC-USDT");
        liq.best_bid = Some(BookLevel { price: dec!(99), quantity: dec!(10) });
        liq.best_ask = Some(BookLevel { price: dec!(101), quantity: dec!(10) });
        liq.ask_depth = vec![
            BookLevel { price: dec!(101), quantity: dec!(5) },
            BookLevel { price: dec!(102), quantity: dec!(5) },
        ];

        // Mid = 100, buy 10 = avg 101.5, impact = 150 bps
        let impact = liq.estimate_impact_bps(Side::Buy, dec!(10)).unwrap();
        assert_eq!(impact, dec!(150));
    }

    #[test]
    fn test_parent_order_creation() {
        let order = ParentOrder::new("BTC-USDT", Side::Buy, dec!(1.0), OrderType::Market)
            .with_urgency(0.8)
            .with_max_venues(3)
            .exclude_venue("bad_venue");

        assert_eq!(order.symbol, "BTC-USDT");
        assert_eq!(order.side, Side::Buy);
        assert_eq!(order.quantity, dec!(1.0));
        assert_eq!(order.urgency, 0.8);
        assert_eq!(order.max_venues, Some(3));
        assert!(order.excluded_venues.contains(&"bad_venue".to_string()));
    }

    #[test]
    fn test_parent_order_limit_price() {
        let market = ParentOrder::new("BTC", Side::Buy, dec!(1), OrderType::Market);
        assert!(market.limit_price().is_none());

        let limit = ParentOrder::new("BTC", Side::Buy, dec!(1), OrderType::Limit(dec!(100)));
        assert_eq!(limit.limit_price(), Some(dec!(100)));
    }

    #[test]
    fn test_child_order_notional() {
        let child = ChildOrder::new("parent1", "binance", "BTC", Side::Buy, dec!(0.5), dec!(100000));
        assert_eq!(child.notional(), dec!(50000));
    }

    #[test]
    fn test_child_order_total_cost() {
        let mut child = ChildOrder::new("p1", "v1", "BTC", Side::Buy, dec!(1), dec!(100));
        child.expected_fee = dec!(0.001);

        // Buy: notional + fee
        assert_eq!(child.total_cost(), dec!(100.1));

        child.side = Side::Sell;
        // Sell: notional - fee
        assert_eq!(child.total_cost(), dec!(99.9));
    }

    #[test]
    fn test_venue_score_calculation() {
        let weights = ScoreWeights::default();
        let mut score = VenueScore::new("test");
        score.price_score = 80.0;
        score.liquidity_score = 70.0;
        score.fee_score = 60.0;
        score.latency_score = 50.0;
        score.reliability_score = 90.0;

        score.calculate_total(&weights);

        // 80*0.35 + 70*0.25 + 60*0.15 + 50*0.15 + 90*0.10 = 28+17.5+9+7.5+9 = 71
        assert!((score.total_score - 71.0).abs() < 0.01);
    }

    #[test]
    fn test_score_weights_for_urgency() {
        let patient = ScoreWeights::for_urgency(0.0);
        let urgent = ScoreWeights::for_urgency(1.0);

        // Patient prefers price, urgent prefers liquidity
        assert!(patient.price > urgent.price);
        assert!(patient.liquidity < urgent.liquidity);
    }

    #[test]
    fn test_router_register_venue() {
        let mut router = SmartOrderRouter::new(RoutingConfig::default());
        assert_eq!(router.venue_count(), 0);

        router.register_venue(Venue::new("v1", "Venue 1"));
        assert_eq!(router.venue_count(), 1);

        router.register_venue(Venue::new("v2", "Venue 2"));
        assert_eq!(router.venue_count(), 2);

        router.remove_venue("v1");
        assert_eq!(router.venue_count(), 1);
    }

    #[test]
    fn test_router_route_best_venue() {
        let mut router = setup_test_router();
        router.set_config(RoutingConfig {
            algorithm: RoutingAlgorithm::BestVenue,
            ..Default::default()
        });

        let order = ParentOrder::new("BTC-USDT", Side::Buy, dec!(1.0), OrderType::Market);
        let decision = router.route(&order);

        assert!(!decision.child_orders.is_empty());
        assert_eq!(decision.num_venues(), 1);
        assert!(decision.coverage > Decimal::ZERO);
    }

    #[test]
    fn test_router_route_pro_rata() {
        let mut router = setup_test_router();
        router.set_config(RoutingConfig {
            algorithm: RoutingAlgorithm::ProRata,
            min_child_fraction: dec!(0.1),
            ..Default::default()
        });

        let order = ParentOrder::new("BTC-USDT", Side::Buy, dec!(10.0), OrderType::Market);
        let decision = router.route(&order);

        assert!(!decision.child_orders.is_empty());
        // Should use multiple venues
        assert!(decision.num_venues() >= 1);
    }

    #[test]
    fn test_router_route_min_cost() {
        let mut router = setup_test_router();
        router.set_config(RoutingConfig {
            algorithm: RoutingAlgorithm::MinCost,
            ..Default::default()
        });

        let order = ParentOrder::new("BTC-USDT", Side::Buy, dec!(5.0), OrderType::Market);
        let decision = router.route(&order);

        assert!(!decision.child_orders.is_empty());
        assert!(decision.expected_avg_price > Decimal::ZERO);
    }

    #[test]
    fn test_router_route_min_impact() {
        let mut router = setup_test_router();
        router.set_config(RoutingConfig {
            algorithm: RoutingAlgorithm::MinImpact,
            min_child_fraction: dec!(0.05),
            ..Default::default()
        });

        let order = ParentOrder::new("BTC-USDT", Side::Buy, dec!(15.0), OrderType::Market);
        let decision = router.route(&order);

        // Min impact spreads across venues
        assert!(!decision.child_orders.is_empty());
    }

    #[test]
    fn test_router_route_spray() {
        let mut router = setup_test_router();
        router.set_config(RoutingConfig {
            algorithm: RoutingAlgorithm::Spray,
            min_child_fraction: dec!(0.01),
            ..Default::default()
        });

        let order = ParentOrder::new("BTC-USDT", Side::Buy, dec!(9.0), OrderType::Market);
        let decision = router.route(&order);

        // Spray should use all available venues equally
        assert!(decision.num_venues() >= 2);
    }

    #[test]
    fn test_router_excluded_venues() {
        let mut router = setup_test_router();
        router.set_config(RoutingConfig {
            algorithm: RoutingAlgorithm::BestVenue,
            ..Default::default()
        });

        let order = ParentOrder::new("BTC-USDT", Side::Buy, dec!(1.0), OrderType::Market)
            .exclude_venue("binance")
            .exclude_venue("okx");

        let decision = router.route(&order);

        // Should only route to bybit
        assert!(decision.child_orders.iter().all(|c| c.venue_id == "bybit"));
    }

    #[test]
    fn test_router_max_venues_limit() {
        let mut router = setup_test_router();
        router.set_config(RoutingConfig {
            algorithm: RoutingAlgorithm::Spray,
            max_venues: 2,
            min_child_fraction: dec!(0.01),
            ..Default::default()
        });

        let order = ParentOrder::new("BTC-USDT", Side::Buy, dec!(10.0), OrderType::Market);
        let decision = router.route(&order);

        assert!(decision.num_venues() <= 2);
    }

    #[test]
    fn test_router_parent_max_venues() {
        let mut router = setup_test_router();
        router.set_config(RoutingConfig {
            algorithm: RoutingAlgorithm::Spray,
            max_venues: 10,
            min_child_fraction: dec!(0.01),
            ..Default::default()
        });

        let order = ParentOrder::new("BTC-USDT", Side::Buy, dec!(10.0), OrderType::Market)
            .with_max_venues(1);

        let decision = router.route(&order);
        assert!(decision.num_venues() <= 1);
    }

    #[test]
    fn test_router_venue_status_update() {
        let mut router = setup_test_router();

        router.update_venue_status("binance", VenueStatus::Unavailable);

        let order = ParentOrder::new("BTC-USDT", Side::Buy, dec!(1.0), OrderType::Market);
        let decision = router.route(&order);

        // Should not route to unavailable binance
        assert!(decision
            .child_orders
            .iter()
            .all(|c| c.venue_id != "binance"));
    }

    #[test]
    fn test_router_execution_feedback() {
        let mut router = setup_test_router();

        let feedback = ExecutionFeedback {
            child_id: "c1".to_string(),
            venue_id: "binance".to_string(),
            requested_qty: dec!(1.0),
            filled_qty: dec!(1.0),
            requested_price: dec!(100000),
            actual_price: dec!(100005),
            actual_slippage_bps: dec!(0.5),
            latency_ms: 25,
            success: true,
            error: None,
        };

        router.record_feedback(feedback);

        let metrics = router.get_metrics("binance").unwrap();
        assert_eq!(metrics.total_orders, 1);
        assert_eq!(metrics.successful_fills, 1);
    }

    #[test]
    fn test_venue_metrics_update() {
        let mut metrics = VenueMetrics::default();

        // Success
        metrics.update(&ExecutionFeedback {
            child_id: "1".to_string(),
            venue_id: "v".to_string(),
            requested_qty: dec!(1),
            filled_qty: dec!(1),
            requested_price: dec!(100),
            actual_price: dec!(100.1),
            actual_slippage_bps: dec!(10),
            latency_ms: 50,
            success: true,
            error: None,
        });

        assert_eq!(metrics.total_orders, 1);
        assert_eq!(metrics.successful_fills, 1);
        assert!(metrics.fill_rate() > 0.99);

        // Failure
        metrics.update(&ExecutionFeedback {
            child_id: "2".to_string(),
            venue_id: "v".to_string(),
            requested_qty: dec!(1),
            filled_qty: dec!(0),
            requested_price: dec!(100),
            actual_price: dec!(0),
            actual_slippage_bps: dec!(0),
            latency_ms: 100,
            success: false,
            error: Some("timeout".to_string()),
        });

        assert_eq!(metrics.total_orders, 2);
        assert_eq!(metrics.failed_orders, 1);
        assert_eq!(metrics.fill_rate(), 0.5);
    }

    #[test]
    fn test_routing_decision_coverage() {
        let mut router = setup_test_router();

        // Small order should be fully covered
        let order = ParentOrder::new("BTC-USDT", Side::Buy, dec!(1.0), OrderType::Market);
        let decision = router.route(&order);
        assert!(decision.is_fully_covered());

        // Huge order might not be fully covered
        let big_order = ParentOrder::new("BTC-USDT", Side::Buy, dec!(1000.0), OrderType::Market);
        let big_decision = router.route(&big_order);
        // Coverage will be partial since total liquidity is limited
        assert!(big_decision.coverage < dec!(1));
    }

    #[test]
    fn test_routing_algorithm_variants() {
        let mut router = setup_test_router();
        let order = ParentOrder::new("BTC-USDT", Side::Buy, dec!(5.0), OrderType::Market);

        for algo in [
            RoutingAlgorithm::BestVenue,
            RoutingAlgorithm::ProRata,
            RoutingAlgorithm::MinCost,
            RoutingAlgorithm::MinImpact,
            RoutingAlgorithm::Spray,
        ] {
            router.set_config(RoutingConfig {
                algorithm: algo,
                min_child_fraction: dec!(0.01),
                ..Default::default()
            });

            let decision = router.route(&order);
            assert_eq!(decision.algorithm, algo);
            assert!(!decision.child_orders.is_empty());
        }
    }

    #[test]
    fn test_sell_order_routing() {
        let mut router = setup_test_router();

        let order = ParentOrder::new("BTC-USDT", Side::Sell, dec!(5.0), OrderType::Market);
        let decision = router.route(&order);

        assert!(!decision.child_orders.is_empty());
        assert!(decision.child_orders.iter().all(|c| c.side == Side::Sell));
    }

    #[test]
    fn test_limit_order_routing() {
        let mut router = setup_test_router();

        let order = ParentOrder::new(
            "BTC-USDT",
            Side::Buy,
            dec!(5.0),
            OrderType::Limit(dec!(100015)),
        );
        let decision = router.route(&order);

        // Should respect limit price
        assert!(!decision.child_orders.is_empty());
    }

    #[test]
    fn test_urgency_affects_weights() {
        let mut router = setup_test_router();

        let patient = ParentOrder::new("BTC-USDT", Side::Buy, dec!(1.0), OrderType::Market)
            .with_urgency(0.0);
        let urgent = ParentOrder::new("BTC-USDT", Side::Buy, dec!(1.0), OrderType::Market)
            .with_urgency(1.0);

        let patient_decision = router.route(&patient);
        let urgent_decision = router.route(&urgent);

        // Both should complete, but may have different venue selection
        assert!(!patient_decision.child_orders.is_empty());
        assert!(!urgent_decision.child_orders.is_empty());
    }

    #[test]
    fn test_compute_time_tracking() {
        let mut router = setup_test_router();

        let order = ParentOrder::new("BTC-USDT", Side::Buy, dec!(1.0), OrderType::Market);
        let decision = router.route(&order);

        // Compute time should be recorded
        assert!(decision.compute_time_us > 0);
    }
}
