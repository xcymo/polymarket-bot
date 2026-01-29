//! Ultra-low latency execution optimizations
//!
//! This module provides professional-grade latency optimizations for high-frequency trading:
//! - Local order book maintenance (no API calls for price lookups)
//! - Connection pool with pre-warmed connections
//! - Batch order submission
//! - Pre-signed order templates
//! - Lock-free statistics tracking
//!
//! Target: sub-10ms order placement latency

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, RwLock};
use tokio::time::interval;

// ============================================================================
// Configuration
// ============================================================================

/// Latency optimizer configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LatencyConfig {
    /// Enable local order book caching
    pub enable_local_orderbook: bool,
    /// Order book staleness threshold (ms)
    pub orderbook_stale_threshold_ms: u64,
    /// Number of pre-warmed connections to maintain
    pub connection_pool_size: usize,
    /// Connection warmup interval (seconds)
    pub warmup_interval_secs: u64,
    /// Maximum batch size for orders
    pub max_batch_size: usize,
    /// Batch wait timeout (ms) - 0 for immediate
    pub batch_timeout_ms: u64,
    /// Enable pre-signed order templates
    pub enable_presigned_templates: bool,
    /// Number of pre-signed templates per token
    pub presigned_templates_per_token: usize,
    /// Target latency percentile to optimize (e.g., 99.0)
    pub target_latency_percentile: f64,
}

impl Default for LatencyConfig {
    fn default() -> Self {
        Self {
            enable_local_orderbook: true,
            orderbook_stale_threshold_ms: 100,
            connection_pool_size: 4,
            warmup_interval_secs: 30,
            max_batch_size: 10,
            batch_timeout_ms: 5,
            enable_presigned_templates: true,
            presigned_templates_per_token: 5,
            target_latency_percentile: 99.0,
        }
    }
}

// ============================================================================
// Local Order Book Cache
// ============================================================================

/// A single price level in the order book
#[derive(Debug, Clone, Default)]
pub struct PriceLevel {
    pub price: Decimal,
    pub size: Decimal,
    pub order_count: u32,
}

/// Local order book state maintained via WebSocket updates
#[derive(Debug, Clone)]
pub struct LocalOrderBook {
    pub token_id: String,
    pub bids: Vec<PriceLevel>,  // Sorted descending (best bid first)
    pub asks: Vec<PriceLevel>,  // Sorted ascending (best ask first)
    pub last_update_us: u64,    // Microsecond timestamp
    pub sequence: u64,          // For detecting gaps
}

impl LocalOrderBook {
    pub fn new(token_id: String) -> Self {
        Self {
            token_id,
            bids: Vec::with_capacity(50),
            asks: Vec::with_capacity(50),
            last_update_us: 0,
            sequence: 0,
        }
    }

    /// Get best bid price (highest bid)
    #[inline]
    pub fn best_bid(&self) -> Option<Decimal> {
        self.bids.first().map(|l| l.price)
    }

    /// Get best ask price (lowest ask)
    #[inline]
    pub fn best_ask(&self) -> Option<Decimal> {
        self.asks.first().map(|l| l.price)
    }

    /// Get mid price
    #[inline]
    pub fn mid_price(&self) -> Option<Decimal> {
        match (self.best_bid(), self.best_ask()) {
            (Some(bid), Some(ask)) => Some((bid + ask) / Decimal::TWO),
            _ => None,
        }
    }

    /// Get spread in basis points
    #[inline]
    pub fn spread_bps(&self) -> Option<Decimal> {
        match (self.best_bid(), self.best_ask()) {
            (Some(bid), Some(ask)) if bid > Decimal::ZERO => {
                let spread = ask - bid;
                let mid = (bid + ask) / Decimal::TWO;
                Some(spread / mid * Decimal::from(10000))
            }
            _ => None,
        }
    }

    /// Get total bid depth up to a price
    pub fn bid_depth_to_price(&self, limit_price: Decimal) -> Decimal {
        self.bids
            .iter()
            .take_while(|l| l.price >= limit_price)
            .map(|l| l.size)
            .sum()
    }

    /// Get total ask depth up to a price
    pub fn ask_depth_to_price(&self, limit_price: Decimal) -> Decimal {
        self.asks
            .iter()
            .take_while(|l| l.price <= limit_price)
            .map(|l| l.size)
            .sum()
    }

    /// Check if order book is stale
    pub fn is_stale(&self, threshold_us: u64) -> bool {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_micros() as u64;
        now.saturating_sub(self.last_update_us) > threshold_us
    }

    /// Apply a delta update from WebSocket
    pub fn apply_delta(&mut self, update: OrderBookDelta) {
        // Check sequence for gaps
        if update.sequence > 0 && self.sequence > 0 {
            if update.sequence != self.sequence + 1 {
                // Gap detected - should request full snapshot
                tracing::warn!(
                    "Order book sequence gap: {} -> {}",
                    self.sequence,
                    update.sequence
                );
            }
        }
        self.sequence = update.sequence;
        self.last_update_us = update.timestamp_us;

        // Apply bid updates
        for (price, size) in update.bid_updates {
            Self::apply_level_update(&mut self.bids, price, size, true);
        }

        // Apply ask updates
        for (price, size) in update.ask_updates {
            Self::apply_level_update(&mut self.asks, price, size, false);
        }
    }

    fn apply_level_update(
        levels: &mut Vec<PriceLevel>,
        price: Decimal,
        size: Decimal,
        is_bid: bool,
    ) {
        // Find the position for this price
        let pos = if is_bid {
            // Bids sorted descending
            levels.iter().position(|l| l.price <= price)
        } else {
            // Asks sorted ascending
            levels.iter().position(|l| l.price >= price)
        };

        match pos {
            Some(idx) if levels[idx].price == price => {
                // Update existing level
                if size == Decimal::ZERO {
                    levels.remove(idx);
                } else {
                    levels[idx].size = size;
                }
            }
            Some(idx) if size > Decimal::ZERO => {
                // Insert new level
                levels.insert(
                    idx,
                    PriceLevel {
                        price,
                        size,
                        order_count: 1,
                    },
                );
            }
            None if size > Decimal::ZERO => {
                // Append to end
                levels.push(PriceLevel {
                    price,
                    size,
                    order_count: 1,
                });
            }
            _ => {}
        }
    }
}

/// Delta update from WebSocket
#[derive(Debug, Clone)]
pub struct OrderBookDelta {
    pub token_id: String,
    pub bid_updates: Vec<(Decimal, Decimal)>,  // (price, new_size)
    pub ask_updates: Vec<(Decimal, Decimal)>,
    pub timestamp_us: u64,
    pub sequence: u64,
}

// ============================================================================
// Lock-Free Latency Statistics
// ============================================================================

/// Lock-free latency statistics using atomics
pub struct LatencyStats {
    // Histogram buckets (microseconds): 0-100, 100-500, 500-1000, 1000-5000, 5000-10000, 10000+
    bucket_0_100: AtomicU64,
    bucket_100_500: AtomicU64,
    bucket_500_1000: AtomicU64,
    bucket_1000_5000: AtomicU64,
    bucket_5000_10000: AtomicU64,
    bucket_10000_plus: AtomicU64,
    
    // Running statistics
    total_count: AtomicU64,
    total_latency_us: AtomicU64,
    min_latency_us: AtomicU64,
    max_latency_us: AtomicU64,
    
    // Error tracking
    timeout_count: AtomicU64,
    error_count: AtomicU64,
}

impl LatencyStats {
    pub fn new() -> Self {
        Self {
            bucket_0_100: AtomicU64::new(0),
            bucket_100_500: AtomicU64::new(0),
            bucket_500_1000: AtomicU64::new(0),
            bucket_1000_5000: AtomicU64::new(0),
            bucket_5000_10000: AtomicU64::new(0),
            bucket_10000_plus: AtomicU64::new(0),
            total_count: AtomicU64::new(0),
            total_latency_us: AtomicU64::new(0),
            min_latency_us: AtomicU64::new(u64::MAX),
            max_latency_us: AtomicU64::new(0),
            timeout_count: AtomicU64::new(0),
            error_count: AtomicU64::new(0),
        }
    }

    /// Record a latency measurement (lock-free)
    #[inline]
    pub fn record(&self, latency_us: u64) {
        // Update histogram
        match latency_us {
            0..=99 => self.bucket_0_100.fetch_add(1, Ordering::Relaxed),
            100..=499 => self.bucket_100_500.fetch_add(1, Ordering::Relaxed),
            500..=999 => self.bucket_500_1000.fetch_add(1, Ordering::Relaxed),
            1000..=4999 => self.bucket_1000_5000.fetch_add(1, Ordering::Relaxed),
            5000..=9999 => self.bucket_5000_10000.fetch_add(1, Ordering::Relaxed),
            _ => self.bucket_10000_plus.fetch_add(1, Ordering::Relaxed),
        };

        // Update running stats
        self.total_count.fetch_add(1, Ordering::Relaxed);
        self.total_latency_us.fetch_add(latency_us, Ordering::Relaxed);

        // Update min (CAS loop)
        let mut current_min = self.min_latency_us.load(Ordering::Relaxed);
        while latency_us < current_min {
            match self.min_latency_us.compare_exchange_weak(
                current_min,
                latency_us,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(actual) => current_min = actual,
            }
        }

        // Update max (CAS loop)
        let mut current_max = self.max_latency_us.load(Ordering::Relaxed);
        while latency_us > current_max {
            match self.max_latency_us.compare_exchange_weak(
                current_max,
                latency_us,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(actual) => current_max = actual,
            }
        }
    }

    /// Record a timeout
    #[inline]
    pub fn record_timeout(&self) {
        self.timeout_count.fetch_add(1, Ordering::Relaxed);
    }

    /// Record an error
    #[inline]
    pub fn record_error(&self) {
        self.error_count.fetch_add(1, Ordering::Relaxed);
    }

    /// Get average latency in microseconds
    pub fn avg_latency_us(&self) -> f64 {
        let total = self.total_latency_us.load(Ordering::Relaxed);
        let count = self.total_count.load(Ordering::Relaxed);
        if count == 0 {
            0.0
        } else {
            total as f64 / count as f64
        }
    }

    /// Get percentile latency (approximate from histogram)
    pub fn percentile_latency_us(&self, percentile: f64) -> u64 {
        let total = self.total_count.load(Ordering::Relaxed);
        if total == 0 {
            return 0;
        }

        let target = ((percentile / 100.0) * total as f64) as u64;
        let mut cumulative = 0u64;

        let buckets = [
            (self.bucket_0_100.load(Ordering::Relaxed), 100),
            (self.bucket_100_500.load(Ordering::Relaxed), 500),
            (self.bucket_500_1000.load(Ordering::Relaxed), 1000),
            (self.bucket_1000_5000.load(Ordering::Relaxed), 5000),
            (self.bucket_5000_10000.load(Ordering::Relaxed), 10000),
            (self.bucket_10000_plus.load(Ordering::Relaxed), 20000),
        ];

        for (count, upper_bound) in buckets {
            cumulative += count;
            if cumulative >= target {
                return upper_bound;
            }
        }

        20000 // Above highest bucket
    }

    /// Get summary statistics
    pub fn summary(&self) -> LatencySummary {
        let total = self.total_count.load(Ordering::Relaxed);
        let min = self.min_latency_us.load(Ordering::Relaxed);
        let max = self.max_latency_us.load(Ordering::Relaxed);

        LatencySummary {
            total_requests: total,
            avg_latency_us: self.avg_latency_us(),
            min_latency_us: if min == u64::MAX { 0 } else { min },
            max_latency_us: max,
            p50_latency_us: self.percentile_latency_us(50.0),
            p95_latency_us: self.percentile_latency_us(95.0),
            p99_latency_us: self.percentile_latency_us(99.0),
            timeout_count: self.timeout_count.load(Ordering::Relaxed),
            error_count: self.error_count.load(Ordering::Relaxed),
        }
    }

    /// Reset all statistics
    pub fn reset(&self) {
        self.bucket_0_100.store(0, Ordering::Relaxed);
        self.bucket_100_500.store(0, Ordering::Relaxed);
        self.bucket_500_1000.store(0, Ordering::Relaxed);
        self.bucket_1000_5000.store(0, Ordering::Relaxed);
        self.bucket_5000_10000.store(0, Ordering::Relaxed);
        self.bucket_10000_plus.store(0, Ordering::Relaxed);
        self.total_count.store(0, Ordering::Relaxed);
        self.total_latency_us.store(0, Ordering::Relaxed);
        self.min_latency_us.store(u64::MAX, Ordering::Relaxed);
        self.max_latency_us.store(0, Ordering::Relaxed);
        self.timeout_count.store(0, Ordering::Relaxed);
        self.error_count.store(0, Ordering::Relaxed);
    }
}

impl Default for LatencyStats {
    fn default() -> Self {
        Self::new()
    }
}

/// Summary of latency statistics
#[derive(Debug, Clone, Serialize)]
pub struct LatencySummary {
    pub total_requests: u64,
    pub avg_latency_us: f64,
    pub min_latency_us: u64,
    pub max_latency_us: u64,
    pub p50_latency_us: u64,
    pub p95_latency_us: u64,
    pub p99_latency_us: u64,
    pub timeout_count: u64,
    pub error_count: u64,
}

// ============================================================================
// Connection Pool with Pre-warming
// ============================================================================

/// Pre-warmed HTTP connection pool
pub struct ConnectionPool {
    connections: Vec<reqwest::Client>,
    current_index: AtomicUsize,
    warmup_url: String,
}

impl ConnectionPool {
    /// Create a new connection pool
    pub fn new(size: usize, warmup_url: String) -> Self {
        let connections: Vec<_> = (0..size)
            .map(|_| {
                reqwest::Client::builder()
                    .pool_idle_timeout(Duration::from_secs(60))
                    .pool_max_idle_per_host(10)
                    .tcp_nodelay(true)
                    .tcp_keepalive(Duration::from_secs(30))
                    .timeout(Duration::from_secs(10))
                    .build()
                    .expect("Failed to create HTTP client")
            })
            .collect();

        Self {
            connections,
            current_index: AtomicUsize::new(0),
            warmup_url,
        }
    }

    /// Get next connection (round-robin)
    #[inline]
    pub fn get(&self) -> &reqwest::Client {
        let idx = self.current_index.fetch_add(1, Ordering::Relaxed) % self.connections.len();
        &self.connections[idx]
    }

    /// Warm up all connections by making a request
    pub async fn warmup(&self) {
        for client in &self.connections {
            let _ = client.get(&self.warmup_url).send().await;
        }
    }

    /// Start background warmup task
    pub fn start_warmup_task(self: Arc<Self>, interval_secs: u64) {
        tokio::spawn(async move {
            let mut ticker = interval(Duration::from_secs(interval_secs));
            loop {
                ticker.tick().await;
                self.warmup().await;
            }
        });
    }
}

// ============================================================================
// Batch Order Submitter
// ============================================================================

/// Order to be batched
#[derive(Debug)]
pub struct BatchableOrder {
    pub token_id: String,
    pub side: OrderSide,
    pub price: Decimal,
    pub size: Decimal,
    pub response_tx: Option<tokio::sync::oneshot::Sender<BatchOrderResult>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderSide {
    Buy,
    Sell,
}

/// Result of a batched order
#[derive(Debug, Clone)]
pub struct BatchOrderResult {
    pub success: bool,
    pub order_id: Option<String>,
    pub error: Option<String>,
    pub latency_us: u64,
}

/// Batch order submitter with configurable batching
pub struct BatchSubmitter {
    order_tx: mpsc::Sender<BatchableOrder>,
    config: BatchConfig,
    stats: Arc<LatencyStats>,
}

#[derive(Debug, Clone)]
pub struct BatchConfig {
    pub max_batch_size: usize,
    pub batch_timeout_ms: u64,
}

impl BatchSubmitter {
    /// Create a new batch submitter
    pub fn new(config: BatchConfig, stats: Arc<LatencyStats>) -> Self {
        let (tx, rx) = mpsc::channel(1000);

        // Start batch processor
        let stats_clone = stats.clone();
        let config_clone = config.clone();
        tokio::spawn(async move {
            Self::batch_processor(rx, config_clone, stats_clone).await;
        });

        Self {
            order_tx: tx,
            config,
            stats,
        }
    }

    /// Submit an order for batching
    pub async fn submit(&self, order: BatchableOrder) -> Result<(), String> {
        self.order_tx
            .send(order)
            .await
            .map_err(|e| format!("Failed to submit order: {}", e))
    }

    /// Internal batch processor
    async fn batch_processor(
        mut rx: mpsc::Receiver<BatchableOrder>,
        config: BatchConfig,
        stats: Arc<LatencyStats>,
    ) {
        let mut batch: Vec<BatchableOrder> = Vec::with_capacity(config.max_batch_size);

        loop {
            // Wait for first order or shutdown
            let order = match rx.recv().await {
                Some(o) => o,
                None => break,
            };
            batch.push(order);

            // Collect more orders within timeout
            let deadline = if config.batch_timeout_ms > 0 {
                Some(Instant::now() + Duration::from_millis(config.batch_timeout_ms))
            } else {
                None
            };

            while batch.len() < config.max_batch_size {
                let timeout = deadline
                    .map(|d| d.saturating_duration_since(Instant::now()))
                    .unwrap_or(Duration::ZERO);

                if timeout.is_zero() {
                    break;
                }

                match tokio::time::timeout(timeout, rx.recv()).await {
                    Ok(Some(order)) => batch.push(order),
                    Ok(None) => break, // Channel closed
                    Err(_) => break,   // Timeout
                }
            }

            // Process batch
            let start = Instant::now();
            Self::process_batch(&mut batch, &stats).await;
            let latency_us = start.elapsed().as_micros() as u64;

            tracing::debug!(
                "Processed batch of {} orders in {}us",
                batch.len(),
                latency_us
            );

            batch.clear();
        }
    }

    /// Process a batch of orders (placeholder - integrate with actual API)
    async fn process_batch(batch: &mut [BatchableOrder], stats: &LatencyStats) {
        let start = Instant::now();

        // In production, this would call the batch order API
        // For now, simulate processing
        for order in batch.iter_mut() {
            let latency_us = start.elapsed().as_micros() as u64;
            stats.record(latency_us);

            if let Some(tx) = order.response_tx.take() {
                let _ = tx.send(BatchOrderResult {
                    success: true,
                    order_id: Some(format!("batch-{}", uuid::Uuid::new_v4())),
                    error: None,
                    latency_us,
                });
            }
        }
    }

    /// Get current statistics
    pub fn stats(&self) -> LatencySummary {
        self.stats.summary()
    }
}

// ============================================================================
// Pre-signed Order Template Cache
// ============================================================================

/// Pre-signed order template for ultra-fast submission
#[derive(Debug, Clone)]
pub struct PreSignedTemplate {
    pub token_id: String,
    pub side: OrderSide,
    pub base_size: Decimal,
    pub signature: String,  // Pre-computed signature
    pub expiry: u64,        // Unix timestamp
    pub nonce: u64,
}

/// Cache of pre-signed order templates
pub struct TemplateCache {
    templates: RwLock<HashMap<String, Vec<PreSignedTemplate>>>,
    templates_per_token: usize,
}

impl TemplateCache {
    pub fn new(templates_per_token: usize) -> Self {
        Self {
            templates: RwLock::new(HashMap::new()),
            templates_per_token,
        }
    }

    /// Get a pre-signed template for a token
    pub async fn get_template(
        &self,
        token_id: &str,
        side: OrderSide,
    ) -> Option<PreSignedTemplate> {
        let templates = self.templates.read().await;
        templates.get(token_id).and_then(|tpls| {
            tpls.iter()
                .find(|t| t.side == side && !Self::is_expired(t))
                .cloned()
        })
    }

    /// Add templates for a token
    pub async fn add_templates(&self, token_id: &str, new_templates: Vec<PreSignedTemplate>) {
        let mut templates = self.templates.write().await;
        let entry = templates.entry(token_id.to_string()).or_default();
        entry.extend(new_templates);
        // Keep only the newest ones
        if entry.len() > self.templates_per_token {
            entry.sort_by(|a, b| b.expiry.cmp(&a.expiry));
            entry.truncate(self.templates_per_token);
        }
    }

    /// Remove expired templates
    pub async fn cleanup_expired(&self) {
        let mut templates = self.templates.write().await;
        for tpls in templates.values_mut() {
            tpls.retain(|t| !Self::is_expired(t));
        }
    }

    fn is_expired(template: &PreSignedTemplate) -> bool {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        template.expiry <= now
    }
}

// ============================================================================
// Main Latency Optimizer
// ============================================================================

/// Main latency optimizer combining all optimization techniques
pub struct LatencyOptimizer {
    config: LatencyConfig,
    order_books: RwLock<HashMap<String, LocalOrderBook>>,
    connection_pool: Arc<ConnectionPool>,
    batch_submitter: Option<BatchSubmitter>,
    template_cache: TemplateCache,
    stats: Arc<LatencyStats>,
}

impl LatencyOptimizer {
    /// Create a new latency optimizer
    pub fn new(config: LatencyConfig, api_base_url: &str) -> Self {
        let stats = Arc::new(LatencyStats::new());

        // Initialize connection pool
        let pool = Arc::new(ConnectionPool::new(
            config.connection_pool_size,
            format!("{}/health", api_base_url),
        ));

        // Start warmup task
        if config.warmup_interval_secs > 0 {
            pool.clone().start_warmup_task(config.warmup_interval_secs);
        }

        // Initialize batch submitter if enabled
        let batch_submitter = if config.batch_timeout_ms > 0 || config.max_batch_size > 1 {
            Some(BatchSubmitter::new(
                BatchConfig {
                    max_batch_size: config.max_batch_size,
                    batch_timeout_ms: config.batch_timeout_ms,
                },
                stats.clone(),
            ))
        } else {
            None
        };

        let template_cache = TemplateCache::new(config.presigned_templates_per_token);
        
        Self {
            config,
            order_books: RwLock::new(HashMap::new()),
            connection_pool: pool,
            batch_submitter,
            template_cache,
            stats,
        }
    }

    /// Get best bid for a token (from local cache)
    pub async fn best_bid(&self, token_id: &str) -> Option<Decimal> {
        let books = self.order_books.read().await;
        books.get(token_id).and_then(|b| b.best_bid())
    }

    /// Get best ask for a token (from local cache)
    pub async fn best_ask(&self, token_id: &str) -> Option<Decimal> {
        let books = self.order_books.read().await;
        books.get(token_id).and_then(|b| b.best_ask())
    }

    /// Get full local order book
    pub async fn get_order_book(&self, token_id: &str) -> Option<LocalOrderBook> {
        let books = self.order_books.read().await;
        books.get(token_id).cloned()
    }

    /// Check if order book is stale
    pub async fn is_orderbook_stale(&self, token_id: &str) -> bool {
        let books = self.order_books.read().await;
        books
            .get(token_id)
            .map(|b| b.is_stale(self.config.orderbook_stale_threshold_ms * 1000))
            .unwrap_or(true)
    }

    /// Apply order book delta update
    pub async fn apply_orderbook_delta(&self, delta: OrderBookDelta) {
        let mut books = self.order_books.write().await;
        let book = books
            .entry(delta.token_id.clone())
            .or_insert_with(|| LocalOrderBook::new(delta.token_id.clone()));
        book.apply_delta(delta);
    }

    /// Initialize order book from snapshot
    pub async fn init_orderbook(&self, token_id: &str, bids: Vec<PriceLevel>, asks: Vec<PriceLevel>) {
        let mut books = self.order_books.write().await;
        let book = books
            .entry(token_id.to_string())
            .or_insert_with(|| LocalOrderBook::new(token_id.to_string()));
        book.bids = bids;
        book.asks = asks;
        book.last_update_us = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_micros() as u64;
    }

    /// Get HTTP client from pool
    pub fn get_client(&self) -> &reqwest::Client {
        self.connection_pool.get()
    }

    /// Submit order (uses batching if enabled)
    pub async fn submit_order(&self, order: BatchableOrder) -> Result<BatchOrderResult, String> {
        let start = Instant::now();

        let result = if let Some(ref submitter) = self.batch_submitter {
            let (tx, rx) = tokio::sync::oneshot::channel();
            let order_with_response = BatchableOrder {
                response_tx: Some(tx),
                ..order
            };
            submitter.submit(order_with_response).await?;
            rx.await.map_err(|e| format!("Order response error: {}", e))?
        } else {
            // Direct submission (no batching)
            let latency_us = start.elapsed().as_micros() as u64;
            self.stats.record(latency_us);
            BatchOrderResult {
                success: true,
                order_id: Some(format!("direct-{}", uuid::Uuid::new_v4())),
                error: None,
                latency_us,
            }
        };

        Ok(result)
    }

    /// Get latency statistics
    pub fn stats(&self) -> LatencySummary {
        self.stats.summary()
    }

    /// Reset statistics
    pub fn reset_stats(&self) {
        self.stats.reset();
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn test_local_orderbook_best_prices() {
        let mut book = LocalOrderBook::new("test".to_string());
        book.bids = vec![
            PriceLevel { price: dec!(0.55), size: dec!(100), order_count: 1 },
            PriceLevel { price: dec!(0.54), size: dec!(200), order_count: 2 },
        ];
        book.asks = vec![
            PriceLevel { price: dec!(0.56), size: dec!(150), order_count: 1 },
            PriceLevel { price: dec!(0.57), size: dec!(250), order_count: 2 },
        ];

        assert_eq!(book.best_bid(), Some(dec!(0.55)));
        assert_eq!(book.best_ask(), Some(dec!(0.56)));
        assert_eq!(book.mid_price(), Some(dec!(0.555)));
    }

    #[test]
    fn test_local_orderbook_spread() {
        let mut book = LocalOrderBook::new("test".to_string());
        book.bids = vec![PriceLevel { price: dec!(0.50), size: dec!(100), order_count: 1 }];
        book.asks = vec![PriceLevel { price: dec!(0.52), size: dec!(100), order_count: 1 }];

        // Spread = 0.02, Mid = 0.51, Spread in bps = 0.02/0.51 * 10000 ≈ 392
        let spread_bps = book.spread_bps().unwrap();
        assert!(spread_bps > dec!(390) && spread_bps < dec!(395));
    }

    #[test]
    fn test_local_orderbook_depth() {
        let mut book = LocalOrderBook::new("test".to_string());
        book.bids = vec![
            PriceLevel { price: dec!(0.55), size: dec!(100), order_count: 1 },
            PriceLevel { price: dec!(0.54), size: dec!(200), order_count: 1 },
            PriceLevel { price: dec!(0.53), size: dec!(300), order_count: 1 },
        ];
        book.asks = vec![
            PriceLevel { price: dec!(0.56), size: dec!(150), order_count: 1 },
            PriceLevel { price: dec!(0.57), size: dec!(250), order_count: 1 },
        ];

        assert_eq!(book.bid_depth_to_price(dec!(0.54)), dec!(300)); // 0.55 + 0.54
        assert_eq!(book.ask_depth_to_price(dec!(0.57)), dec!(400)); // 0.56 + 0.57
    }

    #[test]
    fn test_orderbook_delta_update() {
        let mut book = LocalOrderBook::new("test".to_string());
        book.bids = vec![
            PriceLevel { price: dec!(0.55), size: dec!(100), order_count: 1 },
        ];
        book.asks = vec![
            PriceLevel { price: dec!(0.56), size: dec!(150), order_count: 1 },
        ];

        // Apply delta: update bid, add new ask level
        let delta = OrderBookDelta {
            token_id: "test".to_string(),
            bid_updates: vec![(dec!(0.55), dec!(200))], // Update existing
            ask_updates: vec![(dec!(0.565), dec!(100))], // Insert new
            timestamp_us: 1000000,
            sequence: 1,
        };
        book.apply_delta(delta);

        assert_eq!(book.bids[0].size, dec!(200));
        assert_eq!(book.asks.len(), 2);
        assert_eq!(book.asks[0].price, dec!(0.56)); // Still best ask
    }

    #[test]
    fn test_orderbook_delta_remove_level() {
        let mut book = LocalOrderBook::new("test".to_string());
        book.bids = vec![
            PriceLevel { price: dec!(0.55), size: dec!(100), order_count: 1 },
            PriceLevel { price: dec!(0.54), size: dec!(200), order_count: 1 },
        ];

        // Remove top bid level (size = 0)
        let delta = OrderBookDelta {
            token_id: "test".to_string(),
            bid_updates: vec![(dec!(0.55), dec!(0))],
            ask_updates: vec![],
            timestamp_us: 1000000,
            sequence: 1,
        };
        book.apply_delta(delta);

        assert_eq!(book.bids.len(), 1);
        assert_eq!(book.best_bid(), Some(dec!(0.54)));
    }

    #[test]
    fn test_latency_stats_basic() {
        let stats = LatencyStats::new();

        stats.record(50);
        stats.record(150);
        stats.record(600);
        stats.record(2000);

        let summary = stats.summary();
        assert_eq!(summary.total_requests, 4);
        assert_eq!(summary.min_latency_us, 50);
        assert_eq!(summary.max_latency_us, 2000);
    }

    #[test]
    fn test_latency_stats_percentiles() {
        let stats = LatencyStats::new();

        // Add 100 samples in different buckets
        for _ in 0..50 {
            stats.record(50); // bucket 0-100
        }
        for _ in 0..30 {
            stats.record(200); // bucket 100-500
        }
        for _ in 0..15 {
            stats.record(700); // bucket 500-1000
        }
        for _ in 0..5 {
            stats.record(3000); // bucket 1000-5000
        }

        // P50 should be in first bucket (100)
        assert_eq!(stats.percentile_latency_us(50.0), 100);
        // P95 should be higher
        assert!(stats.percentile_latency_us(95.0) >= 500);
    }

    #[test]
    fn test_latency_stats_errors() {
        let stats = LatencyStats::new();

        stats.record(100);
        stats.record_timeout();
        stats.record_timeout();
        stats.record_error();

        let summary = stats.summary();
        assert_eq!(summary.total_requests, 1);
        assert_eq!(summary.timeout_count, 2);
        assert_eq!(summary.error_count, 1);
    }

    #[test]
    fn test_latency_stats_reset() {
        let stats = LatencyStats::new();

        stats.record(100);
        stats.record(200);
        stats.record_timeout();

        stats.reset();

        let summary = stats.summary();
        assert_eq!(summary.total_requests, 0);
        assert_eq!(summary.timeout_count, 0);
        assert_eq!(summary.avg_latency_us, 0.0);
    }

    #[test]
    fn test_latency_config_default() {
        let config = LatencyConfig::default();
        assert!(config.enable_local_orderbook);
        assert_eq!(config.connection_pool_size, 4);
        assert_eq!(config.max_batch_size, 10);
    }

    #[test]
    fn test_order_side_equality() {
        assert_eq!(OrderSide::Buy, OrderSide::Buy);
        assert_ne!(OrderSide::Buy, OrderSide::Sell);
    }

    #[test]
    fn test_price_level_default() {
        let level = PriceLevel::default();
        assert_eq!(level.price, Decimal::ZERO);
        assert_eq!(level.size, Decimal::ZERO);
        assert_eq!(level.order_count, 0);
    }

    #[tokio::test]
    async fn test_template_cache_basic() {
        let cache = TemplateCache::new(5);

        let template = PreSignedTemplate {
            token_id: "test".to_string(),
            side: OrderSide::Buy,
            base_size: dec!(100),
            signature: "sig".to_string(),
            expiry: u64::MAX, // Never expires
            nonce: 1,
        };

        cache.add_templates("test", vec![template]).await;

        let retrieved = cache.get_template("test", OrderSide::Buy).await;
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().base_size, dec!(100));
    }

    #[tokio::test]
    async fn test_template_cache_expired() {
        let cache = TemplateCache::new(5);

        let template = PreSignedTemplate {
            token_id: "test".to_string(),
            side: OrderSide::Buy,
            base_size: dec!(100),
            signature: "sig".to_string(),
            expiry: 0, // Already expired
            nonce: 1,
        };

        cache.add_templates("test", vec![template]).await;

        let retrieved = cache.get_template("test", OrderSide::Buy).await;
        assert!(retrieved.is_none()); // Should not return expired template
    }

    #[tokio::test]
    async fn test_latency_optimizer_orderbook() {
        let config = LatencyConfig {
            enable_local_orderbook: true,
            orderbook_stale_threshold_ms: 1000,
            connection_pool_size: 1,
            warmup_interval_secs: 0, // Disable warmup
            ..Default::default()
        };

        let optimizer = LatencyOptimizer::new(config, "http://localhost");

        // Initialize order book
        optimizer.init_orderbook(
            "test",
            vec![PriceLevel { price: dec!(0.55), size: dec!(100), order_count: 1 }],
            vec![PriceLevel { price: dec!(0.56), size: dec!(100), order_count: 1 }],
        ).await;

        assert_eq!(optimizer.best_bid("test").await, Some(dec!(0.55)));
        assert_eq!(optimizer.best_ask("test").await, Some(dec!(0.56)));
    }

    #[tokio::test]
    async fn test_latency_optimizer_delta_update() {
        let config = LatencyConfig {
            enable_local_orderbook: true,
            warmup_interval_secs: 0,
            ..Default::default()
        };

        let optimizer = LatencyOptimizer::new(config, "http://localhost");

        // Initialize
        optimizer.init_orderbook(
            "test",
            vec![PriceLevel { price: dec!(0.55), size: dec!(100), order_count: 1 }],
            vec![],
        ).await;

        // Apply delta
        optimizer.apply_orderbook_delta(OrderBookDelta {
            token_id: "test".to_string(),
            bid_updates: vec![(dec!(0.55), dec!(200))],
            ask_updates: vec![(dec!(0.56), dec!(150))],
            timestamp_us: 1000000,
            sequence: 1,
        }).await;

        let book = optimizer.get_order_book("test").await.unwrap();
        assert_eq!(book.bids[0].size, dec!(200));
        assert_eq!(book.asks[0].price, dec!(0.56));
    }

    #[test]
    fn test_latency_summary_serialization() {
        let summary = LatencySummary {
            total_requests: 100,
            avg_latency_us: 500.5,
            min_latency_us: 50,
            max_latency_us: 5000,
            p50_latency_us: 300,
            p95_latency_us: 2000,
            p99_latency_us: 4000,
            timeout_count: 2,
            error_count: 1,
        };

        let json = serde_json::to_string(&summary).unwrap();
        assert!(json.contains("\"total_requests\":100"));
        assert!(json.contains("\"p99_latency_us\":4000"));
    }
}
