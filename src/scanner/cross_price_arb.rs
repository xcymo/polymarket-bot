//! Cross-Price Arbitrage Scanner
//!
//! Implements the strategy used by top trader k9Q2mX4L8A7ZP3R:
//! - When Up + Down < $1, buy both sides
//! - Guaranteed profit = $1 - (Up + Down)
//! 
//! Example: Buy Up @ 45¢ + Down @ 53¢ = 98¢ total
//! When market settles, one side = $1, profit = 2¢ per pair
//!
//! ## Key Differences from Single-Side Strategy
//! 
//! Single-side: Bet on direction, ~50% win rate
//! Cross-price: Lock in profit regardless of outcome, ~100% win rate
//! 
//! Trade-off: Lower per-trade profit but much higher consistency

use super::crypto_market::{CryptoMarket, CryptoMarketDiscovery, MarketInterval, CRYPTO_SYMBOLS};
use crate::error::Result;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::{debug, info, warn};

/// Cross-price arbitrage opportunity
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrossPriceOpp {
    /// Symbol (BTC, ETH, etc.)
    pub symbol: String,
    /// Market slug
    pub slug: String,
    /// Condition ID
    pub condition_id: String,
    /// Up (Yes) token ID
    pub up_token_id: String,
    /// Down (No) token ID
    pub down_token_id: String,
    /// Up price
    pub up_price: Decimal,
    /// Down price
    pub down_price: Decimal,
    /// Total cost (up + down)
    pub total_cost: Decimal,
    /// Spread (1.0 - total_cost) - this is the profit margin
    pub spread: Decimal,
    /// Profit per $1 wagered
    pub profit_per_dollar: Decimal,
    /// Expected profit if executing $X
    pub expected_profit_usd: Decimal,
    /// Market end time
    pub end_time: DateTime<Utc>,
    /// Seconds until settlement
    pub seconds_remaining: i64,
    /// Detection timestamp
    pub detected_at: DateTime<Utc>,
}

impl CrossPriceOpp {
    /// Calculate profit for a given investment
    pub fn calculate_profit(&self, investment: Decimal) -> Decimal {
        // Investment buys (investment / total_cost) pairs
        // Each pair returns $1 when settled
        // Profit = pairs * $1 - investment = investment * spread / total_cost
        if self.total_cost == Decimal::ZERO {
            return Decimal::ZERO;
        }
        investment * self.spread / self.total_cost
    }

    /// Check if opportunity is still valid
    pub fn is_valid(&self) -> bool {
        self.seconds_remaining > 0 && self.spread > Decimal::ZERO
    }

    /// Optimal position sizing (buy equal value of both sides)
    pub fn optimal_allocation(&self, max_position: Decimal) -> (Decimal, Decimal) {
        // Buy Up and Down in proportion to their prices
        // This ensures we get equal number of both tokens
        let num_pairs = max_position / self.total_cost;
        let up_amount = num_pairs * self.up_price;
        let down_amount = num_pairs * self.down_price;
        (up_amount, down_amount)
    }
}

/// Configuration for cross-price arbitrage
#[derive(Debug, Clone)]
pub struct CrossPriceConfig {
    /// Minimum spread to consider (e.g., 0.01 = 1%)
    pub min_spread: Decimal,
    /// Maximum spread to consider (too good = suspicious)
    pub max_spread: Decimal,
    /// Minimum time remaining (seconds)
    pub min_time_remaining: i64,
    /// Maximum time remaining (don't enter too early)
    pub max_time_remaining: i64,
    /// Maximum position per trade (USD)
    pub max_position: Decimal,
    /// Fee rate (0 on Polymarket currently)
    pub fee_rate: Decimal,
}

impl Default for CrossPriceConfig {
    fn default() -> Self {
        Self {
            min_spread: dec!(0.01),        // 1% minimum profit
            max_spread: dec!(0.10),        // 10% max (likely data issue if higher)
            min_time_remaining: 60,        // At least 1 minute left
            max_time_remaining: 600,       // Enter within last 10 minutes
            max_position: dec!(100),       // $100 max per trade
            fee_rate: Decimal::ZERO,       // No fees currently
        }
    }
}

/// Scanner statistics
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ScannerStats {
    pub total_scans: u64,
    pub opportunities_found: u64,
    pub trades_executed: u64,
    pub total_invested: Decimal,
    pub total_profit: Decimal,
    pub avg_spread: Decimal,
    pub best_spread: Decimal,
    pub worst_spread: Decimal,
    pub win_count: u64,
    pub loss_count: u64,
}

impl ScannerStats {
    pub fn win_rate(&self) -> Decimal {
        if self.trades_executed == 0 {
            return Decimal::ZERO;
        }
        Decimal::from(self.win_count) / Decimal::from(self.trades_executed) * dec!(100)
    }

    pub fn roi(&self) -> Decimal {
        if self.total_invested == Decimal::ZERO {
            return Decimal::ZERO;
        }
        self.total_profit / self.total_invested * dec!(100)
    }
}

/// Cross-price arbitrage scanner
pub struct CrossPriceScanner {
    config: CrossPriceConfig,
    discovery: CryptoMarketDiscovery,
    stats: ScannerStats,
    /// Recent opportunities by condition_id
    recent_opps: HashMap<String, CrossPriceOpp>,
}

impl CrossPriceScanner {
    /// Create a new scanner
    pub fn new(gamma_url: &str, config: CrossPriceConfig) -> Self {
        Self {
            config,
            discovery: CryptoMarketDiscovery::new(gamma_url),
            stats: ScannerStats::default(),
            recent_opps: HashMap::new(),
        }
    }

    /// Create with default config
    pub fn with_defaults(gamma_url: &str) -> Self {
        Self::new(gamma_url, CrossPriceConfig::default())
    }

    /// Get scanner stats
    pub fn stats(&self) -> &ScannerStats {
        &self.stats
    }

    /// Get recent opportunities
    pub fn recent_opportunities(&self) -> Vec<&CrossPriceOpp> {
        self.recent_opps.values().collect()
    }

    /// Scan all crypto markets for cross-price arbitrage
    pub async fn scan_all(&mut self) -> Result<Vec<CrossPriceOpp>> {
        let mut opportunities = Vec::new();
        self.stats.total_scans += 1;

        // Scan all symbols across 15m and 30m markets
        for symbol in CRYPTO_SYMBOLS {
            for interval in [MarketInterval::Min15, MarketInterval::Min30] {
                if let Some(opp) = self.scan_market(symbol, interval).await? {
                    opportunities.push(opp);
                }
            }
        }

        // Sort by spread (best opportunities first)
        opportunities.sort_by(|a, b| b.spread.cmp(&a.spread));

        self.stats.opportunities_found += opportunities.len() as u64;
        Ok(opportunities)
    }

    /// Scan a specific market for arbitrage
    async fn scan_market(
        &mut self,
        symbol: &str,
        interval: MarketInterval,
    ) -> Result<Option<CrossPriceOpp>> {
        let market = match self.discovery.get_current_market(symbol, interval).await? {
            Some(m) => m,
            None => return Ok(None),
        };

        // Skip inactive markets
        if !market.active {
            return Ok(None);
        }

        let remaining = market.remaining().num_seconds();
        
        // Check time constraints
        if remaining < self.config.min_time_remaining {
            debug!("[CrossPrice] {} - too late ({} sec remaining)", market.slug, remaining);
            return Ok(None);
        }
        if remaining > self.config.max_time_remaining {
            debug!("[CrossPrice] {} - too early ({} sec remaining)", market.slug, remaining);
            return Ok(None);
        }

        // Calculate spread
        let total_cost = market.up_price + market.down_price;
        let spread = Decimal::ONE - total_cost;

        // Check spread constraints
        if spread < self.config.min_spread {
            debug!("[CrossPrice] {} - spread too low ({:.2}%)", market.slug, spread * dec!(100));
            return Ok(None);
        }
        if spread > self.config.max_spread {
            warn!("[CrossPrice] {} - spread suspiciously high ({:.2}%)", market.slug, spread * dec!(100));
            return Ok(None);
        }

        // Found valid opportunity!
        let opp = CrossPriceOpp {
            symbol: symbol.to_uppercase(),
            slug: market.slug.clone(),
            condition_id: market.condition_id.clone(),
            up_token_id: market.up_token_id.clone(),
            down_token_id: market.down_token_id.clone(),
            up_price: market.up_price,
            down_price: market.down_price,
            total_cost,
            spread,
            profit_per_dollar: spread / total_cost,
            expected_profit_usd: self.config.max_position * spread / total_cost,
            end_time: market.end_time,
            seconds_remaining: remaining,
            detected_at: Utc::now(),
        };

        // Update stats
        if self.stats.best_spread < spread {
            self.stats.best_spread = spread;
        }
        if self.stats.worst_spread == Decimal::ZERO || self.stats.worst_spread > spread {
            self.stats.worst_spread = spread;
        }

        // Cache for deduplication
        self.recent_opps.insert(market.condition_id.clone(), opp.clone());

        info!(
            "[CrossPrice] 🎯 {} - Up: {:.1}¢ + Down: {:.1}¢ = {:.1}¢ | Profit: {:.2}% | {}s left",
            opp.symbol,
            opp.up_price * dec!(100),
            opp.down_price * dec!(100),
            opp.total_cost * dec!(100),
            spread * dec!(100),
            remaining
        );

        Ok(Some(opp))
    }

    /// Record a trade result
    pub fn record_trade(&mut self, invested: Decimal, profit: Decimal, won: bool) {
        self.stats.trades_executed += 1;
        self.stats.total_invested += invested;
        self.stats.total_profit += profit;
        if won {
            self.stats.win_count += 1;
        } else {
            self.stats.loss_count += 1;
        }
    }

    /// Clean up old opportunities
    pub fn cleanup_old(&mut self) {
        let now = Utc::now();
        self.recent_opps.retain(|_, opp| {
            opp.end_time > now
        });
    }

    /// Generate summary report
    pub fn summary(&self) -> String {
        format!(
            "📊 Cross-Price Arbitrage Stats\n\n\
            Scans: {}\n\
            Opportunities: {}\n\
            Trades: {} ({}W/{}L)\n\
            Win Rate: {:.1}%\n\
            Total Invested: ${:.2}\n\
            Total Profit: ${:.2}\n\
            ROI: {:.2}%\n\
            Best Spread: {:.2}%\n",
            self.stats.total_scans,
            self.stats.opportunities_found,
            self.stats.trades_executed,
            self.stats.win_count,
            self.stats.loss_count,
            self.stats.win_rate(),
            self.stats.total_invested,
            self.stats.total_profit,
            self.stats.roi(),
            self.stats.best_spread * dec!(100)
        )
    }
}

/// Paper trading for cross-price arbitrage
#[derive(Debug, Clone)]
pub struct CrossPricePaperTrader {
    balance: Decimal,
    initial_balance: Decimal,
    positions: Vec<PaperPosition>,
    history: Vec<CompletedTrade>,
}

#[derive(Debug, Clone)]
pub struct PaperPosition {
    pub opp: CrossPriceOpp,
    pub invested: Decimal,
    pub up_shares: Decimal,
    pub down_shares: Decimal,
    pub entry_time: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletedTrade {
    pub symbol: String,
    pub slug: String,
    pub invested: Decimal,
    pub returned: Decimal,
    pub profit: Decimal,
    pub spread_at_entry: Decimal,
    pub entry_time: DateTime<Utc>,
    pub exit_time: DateTime<Utc>,
}

impl CrossPricePaperTrader {
    pub fn new(initial_balance: Decimal) -> Self {
        Self {
            balance: initial_balance,
            initial_balance,
            positions: Vec::new(),
            history: Vec::new(),
        }
    }

    /// Execute a paper trade
    pub fn enter(&mut self, opp: &CrossPriceOpp, amount: Decimal) -> bool {
        if amount > self.balance {
            warn!("[Paper] Insufficient balance: ${:.2} < ${:.2}", self.balance, amount);
            return false;
        }

        // Calculate shares
        let num_pairs = amount / opp.total_cost;
        let up_shares = num_pairs;
        let down_shares = num_pairs;

        self.balance -= amount;
        self.positions.push(PaperPosition {
            opp: opp.clone(),
            invested: amount,
            up_shares,
            down_shares,
            entry_time: Utc::now(),
        });

        info!(
            "[Paper] Entered {} - ${:.2} for {:.2} pairs | Spread: {:.2}%",
            opp.symbol, amount, num_pairs, opp.spread * dec!(100)
        );
        true
    }

    /// Settle all expired positions
    pub fn settle(&mut self) {
        let now = Utc::now();
        let (expired, active): (Vec<_>, Vec<_>) = self.positions
            .drain(..)
            .partition(|p| p.opp.end_time <= now);

        self.positions = active;

        for pos in expired {
            // One side always wins, returning $1 per share
            let returned = pos.up_shares; // 1 share = $1 when settled
            let profit = returned - pos.invested;
            
            self.balance += returned;
            
            let trade = CompletedTrade {
                symbol: pos.opp.symbol.clone(),
                slug: pos.opp.slug.clone(),
                invested: pos.invested,
                returned,
                profit,
                spread_at_entry: pos.opp.spread,
                entry_time: pos.entry_time,
                exit_time: now,
            };

            info!(
                "[Paper] Settled {} - Invested: ${:.2}, Returned: ${:.2}, Profit: ${:.4}",
                trade.symbol, trade.invested, trade.returned, trade.profit
            );

            self.history.push(trade);
        }
    }

    /// Get current balance
    pub fn balance(&self) -> Decimal {
        self.balance
    }

    /// Get PnL
    pub fn pnl(&self) -> Decimal {
        self.balance - self.initial_balance + self.positions_value()
    }

    /// Get value of open positions
    pub fn positions_value(&self) -> Decimal {
        self.positions.iter().map(|p| p.invested).sum()
    }

    /// Get trade history
    pub fn history(&self) -> &[CompletedTrade] {
        &self.history
    }

    /// Get summary stats
    pub fn summary(&self) -> String {
        let total_trades = self.history.len();
        let total_profit: Decimal = self.history.iter().map(|t| t.profit).sum();
        let total_invested: Decimal = self.history.iter().map(|t| t.invested).sum();
        let avg_spread = if total_trades > 0 {
            self.history.iter().map(|t| t.spread_at_entry).sum::<Decimal>() 
                / Decimal::from(total_trades as u64)
        } else {
            Decimal::ZERO
        };

        format!(
            "💰 Paper Trading Summary\n\n\
            Initial: ${:.2}\n\
            Current: ${:.2}\n\
            P&L: ${:.4} ({:.2}%)\n\n\
            Completed Trades: {}\n\
            Total Invested: ${:.2}\n\
            Total Profit: ${:.4}\n\
            Avg Spread: {:.2}%\n\
            Open Positions: {}",
            self.initial_balance,
            self.balance,
            self.pnl(),
            if self.initial_balance > Decimal::ZERO { 
                self.pnl() / self.initial_balance * dec!(100) 
            } else { 
                Decimal::ZERO 
            },
            total_trades,
            total_invested,
            total_profit,
            avg_spread * dec!(100),
            self.positions.len()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_opp(symbol: &str, up: Decimal, down: Decimal) -> CrossPriceOpp {
        CrossPriceOpp {
            symbol: symbol.to_string(),
            slug: format!("{}-updown-15m-12345", symbol.to_lowercase()),
            condition_id: "test_condition".to_string(),
            up_token_id: "up_token".to_string(),
            down_token_id: "down_token".to_string(),
            up_price: up,
            down_price: down,
            total_cost: up + down,
            spread: Decimal::ONE - (up + down),
            profit_per_dollar: (Decimal::ONE - (up + down)) / (up + down),
            expected_profit_usd: dec!(100) * (Decimal::ONE - (up + down)) / (up + down),
            end_time: Utc::now() + chrono::Duration::minutes(5),
            seconds_remaining: 300,
            detected_at: Utc::now(),
        }
    }

    #[test]
    fn test_cross_price_opp_calculation() {
        // Target trader example: Up @ 45¢, Down @ 53¢
        let opp = make_opp("BTC", dec!(0.45), dec!(0.53));
        
        assert_eq!(opp.total_cost, dec!(0.98));
        assert_eq!(opp.spread, dec!(0.02));
        
        // $100 investment should yield $2.04 profit
        let profit = opp.calculate_profit(dec!(100));
        assert!(profit > dec!(2));
        assert!(profit < dec!(2.1));
    }

    #[test]
    fn test_optimal_allocation() {
        let opp = make_opp("ETH", dec!(0.40), dec!(0.55));
        let (up_amt, down_amt) = opp.optimal_allocation(dec!(95));
        
        // Should allocate proportionally
        assert!(up_amt > Decimal::ZERO);
        assert!(down_amt > Decimal::ZERO);
        assert!((up_amt + down_amt - dec!(95)).abs() < dec!(0.01));
    }

    #[test]
    fn test_paper_trader_enter() {
        let mut trader = CrossPricePaperTrader::new(dec!(1000));
        let opp = make_opp("SOL", dec!(0.48), dec!(0.50)); // 2% spread
        
        assert!(trader.enter(&opp, dec!(100)));
        assert_eq!(trader.balance(), dec!(900));
        assert_eq!(trader.positions.len(), 1);
    }

    #[test]
    fn test_paper_trader_insufficient_balance() {
        let mut trader = CrossPricePaperTrader::new(dec!(50));
        let opp = make_opp("BTC", dec!(0.45), dec!(0.53));
        
        assert!(!trader.enter(&opp, dec!(100)));
        assert_eq!(trader.balance(), dec!(50));
    }

    #[test]
    fn test_paper_trader_settle() {
        let mut trader = CrossPricePaperTrader::new(dec!(1000));
        
        // Create opportunity that expires in the past
        let mut opp = make_opp("BTC", dec!(0.48), dec!(0.50)); // 2% spread
        opp.end_time = Utc::now() - chrono::Duration::minutes(1);
        
        // Enter position (98¢ per pair)
        trader.enter(&opp, dec!(98));
        
        // Settle - should return $100 (102 pairs * ~$1 each)
        trader.settle();
        
        assert!(trader.positions.is_empty());
        assert_eq!(trader.history.len(), 1);
        
        // Balance should be 902 + 100 = 1002 (minus rounding)
        assert!(trader.balance() > dec!(1001));
    }

    #[test]
    fn test_scanner_stats() {
        let stats = ScannerStats {
            total_scans: 100,
            opportunities_found: 50,
            trades_executed: 20,
            total_invested: dec!(2000),
            total_profit: dec!(40),
            win_count: 20,
            loss_count: 0,
            ..Default::default()
        };
        
        assert_eq!(stats.win_rate(), dec!(100));
        assert_eq!(stats.roi(), dec!(2)); // 40/2000 = 2%
    }

    #[test]
    fn test_config_defaults() {
        let config = CrossPriceConfig::default();
        
        assert_eq!(config.min_spread, dec!(0.01));
        assert_eq!(config.max_spread, dec!(0.10));
        assert_eq!(config.min_time_remaining, 60);
        assert_eq!(config.max_position, dec!(100));
    }

    #[test]
    fn test_opp_validity() {
        let mut opp = make_opp("XRP", dec!(0.45), dec!(0.53));
        assert!(opp.is_valid());
        
        // Invalid spread
        opp.spread = Decimal::ZERO;
        assert!(!opp.is_valid());
        
        // Invalid time
        opp.spread = dec!(0.02);
        opp.seconds_remaining = 0;
        assert!(!opp.is_valid());
    }

    #[test]
    fn test_profit_calculation_edge_cases() {
        // Zero total cost - create manually to avoid division by zero in helper
        let opp = CrossPriceOpp {
            symbol: "TEST".to_string(),
            slug: "test-updown-15m-12345".to_string(),
            condition_id: "test_condition".to_string(),
            up_token_id: "up_token".to_string(),
            down_token_id: "down_token".to_string(),
            up_price: Decimal::ZERO,
            down_price: Decimal::ZERO,
            total_cost: Decimal::ZERO,
            spread: Decimal::ONE,
            profit_per_dollar: Decimal::ZERO,
            expected_profit_usd: Decimal::ZERO,
            end_time: Utc::now() + chrono::Duration::minutes(5),
            seconds_remaining: 300,
            detected_at: Utc::now(),
        };
        assert_eq!(opp.calculate_profit(dec!(100)), Decimal::ZERO);
        
        // Large spread (5%)
        let opp2 = make_opp("BTC", dec!(0.45), dec!(0.50)); // 5% spread
        let profit = opp2.calculate_profit(dec!(100));
        assert!(profit > dec!(5)); // ~5.26% profit
    }

    #[test]
    fn test_paper_trader_pnl() {
        let mut trader = CrossPricePaperTrader::new(dec!(1000));
        
        // After some trades, PnL should reflect changes
        assert_eq!(trader.pnl(), Decimal::ZERO);
        
        let opp = make_opp("ETH", dec!(0.48), dec!(0.50));
        trader.enter(&opp, dec!(100));
        
        // PnL should still be 0 (just moved to position)
        assert_eq!(trader.pnl(), Decimal::ZERO);
    }
}
