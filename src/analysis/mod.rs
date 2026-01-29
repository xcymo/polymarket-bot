//! Trade analysis and pattern recognition
//!
//! Analyzes successful traders to extract strategies:
//! - Entry timing patterns
//! - Position sizing patterns
//! - Exit strategies
//! - Market selection criteria

pub mod pattern;
pub mod trader_profile;

#[cfg(test)]
mod tests;

use crate::types::Side;
use chrono::{DateTime, Timelike, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

/// Trade record for analysis
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeRecord {
    pub trader: String,
    pub market_id: String,
    pub market_question: String,
    pub side: Side,
    pub entry_price: Decimal,
    pub exit_price: Option<Decimal>,
    pub size: Decimal,
    pub entry_time: DateTime<Utc>,
    pub exit_time: Option<DateTime<Utc>>,
    pub pnl: Option<Decimal>,
    pub outcome: Option<TradeOutcome>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub enum TradeOutcome {
    Win,
    Loss,
    Pending,
}

/// Analyzed trading pattern
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradingPattern {
    /// Pattern name
    pub name: String,
    /// Description
    pub description: String,
    /// Win rate when this pattern appears
    pub win_rate: f64,
    /// Average profit when winning
    pub avg_win: Decimal,
    /// Average loss when losing
    pub avg_loss: Decimal,
    /// Expected value
    pub expected_value: Decimal,
    /// Number of samples
    pub sample_count: usize,
    /// Confidence in pattern (0-1)
    pub confidence: f64,
}

/// Strategy insights from analyzing a trader
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraderInsights {
    pub trader: String,
    pub total_trades: usize,
    pub win_rate: f64,
    pub total_pnl: Decimal,
    pub avg_position_size: Decimal,
    pub avg_hold_time_hours: f64,
    
    /// Key patterns discovered
    pub patterns: Vec<TradingPattern>,
    
    /// Market preferences
    pub preferred_categories: Vec<String>,
    
    /// Timing patterns
    pub active_hours: Vec<u8>,  // 0-23
    
    /// Entry characteristics
    pub entry_insights: EntryInsights,
    
    /// Exit characteristics  
    pub exit_insights: ExitInsights,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntryInsights {
    /// Preferred price range for entries
    pub preferred_price_range: (Decimal, Decimal),
    /// Does trader prefer high or low liquidity markets?
    pub liquidity_preference: LiquidityPreference,
    /// Average market age at entry (hours since creation)
    pub avg_market_age_hours: f64,
    /// Often enters before major events?
    pub event_timing: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExitInsights {
    /// Take profit threshold (e.g., exits at 2x)
    pub take_profit_mult: Option<Decimal>,
    /// Stop loss threshold
    pub stop_loss_pct: Option<Decimal>,
    /// Average hold time before profit
    pub avg_win_hold_hours: f64,
    /// Average hold time before loss
    pub avg_loss_hold_hours: f64,
    /// Exits before resolution or holds to end?
    pub early_exit_pct: f64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum LiquidityPreference {
    HighLiquidity,  // >$100k
    MediumLiquidity, // $10k-$100k
    LowLiquidity,   // <$10k
    NoPreference,
}

impl Default for EntryInsights {
    fn default() -> Self {
        Self {
            preferred_price_range: (Decimal::new(20, 2), Decimal::new(80, 2)),
            liquidity_preference: LiquidityPreference::NoPreference,
            avg_market_age_hours: 0.0,
            event_timing: false,
        }
    }
}

impl Default for ExitInsights {
    fn default() -> Self {
        Self {
            take_profit_mult: None,
            stop_loss_pct: None,
            avg_win_hold_hours: 0.0,
            avg_loss_hold_hours: 0.0,
            early_exit_pct: 0.0,
        }
    }
}

/// Main analyzer
pub struct TradeAnalyzer {
    trades: Vec<TradeRecord>,
}

impl TradeAnalyzer {
    pub fn new() -> Self {
        Self { trades: Vec::new() }
    }

    pub fn add_trade(&mut self, trade: TradeRecord) {
        self.trades.push(trade);
    }

    pub fn add_trades(&mut self, trades: Vec<TradeRecord>) {
        self.trades.extend(trades);
    }

    /// Analyze a specific trader's history
    pub fn analyze_trader(&self, trader: &str) -> TraderInsights {
        let trader_trades: Vec<_> = self.trades
            .iter()
            .filter(|t| t.trader == trader)
            .collect();

        if trader_trades.is_empty() {
            return TraderInsights {
                trader: trader.to_string(),
                total_trades: 0,
                win_rate: 0.0,
                total_pnl: Decimal::ZERO,
                avg_position_size: Decimal::ZERO,
                avg_hold_time_hours: 0.0,
                patterns: Vec::new(),
                preferred_categories: Vec::new(),
                active_hours: Vec::new(),
                entry_insights: EntryInsights::default(),
                exit_insights: ExitInsights::default(),
            };
        }

        // Calculate basic stats
        let wins = trader_trades.iter()
            .filter(|t| t.outcome == Some(TradeOutcome::Win))
            .count();
        
        let total_pnl: Decimal = trader_trades.iter()
            .filter_map(|t| t.pnl)
            .sum();

        let avg_size: Decimal = trader_trades.iter()
            .map(|t| t.size)
            .sum::<Decimal>() / Decimal::from(trader_trades.len());

        // Analyze patterns
        let patterns = self.detect_patterns(&trader_trades);
        
        // Active hours
        let mut hour_counts = [0u32; 24];
        for t in &trader_trades {
            hour_counts[t.entry_time.hour() as usize] += 1;
        }
        let active_hours: Vec<u8> = hour_counts.iter()
            .enumerate()
            .filter(|(_, &count)| count > 0)
            .map(|(hour, _)| hour as u8)
            .collect();

        TraderInsights {
            trader: trader.to_string(),
            total_trades: trader_trades.len(),
            win_rate: if trader_trades.is_empty() { 0.0 } else { wins as f64 / trader_trades.len() as f64 },
            total_pnl,
            avg_position_size: avg_size,
            avg_hold_time_hours: 0.0, // TODO: calculate
            patterns,
            preferred_categories: Vec::new(), // TODO: extract from market questions
            active_hours,
            entry_insights: self.analyze_entries(&trader_trades),
            exit_insights: self.analyze_exits(&trader_trades),
        }
    }

    fn detect_patterns(&self, trades: &[&TradeRecord]) -> Vec<TradingPattern> {
        let mut patterns = Vec::new();

        // Pattern 1: "Buy the dip" - enters when price drops >10%
        // Pattern 2: "Momentum" - enters on price increase
        // Pattern 3: "Early bird" - enters early in market lifecycle
        // Pattern 4: "Event trader" - trades around news/events

        // Simplified pattern detection
        let low_price_entries: Vec<_> = trades.iter()
            .filter(|t| t.entry_price < Decimal::new(30, 2))
            .collect();

        if low_price_entries.len() > 3 {
            let wins = low_price_entries.iter()
                .filter(|t| t.outcome == Some(TradeOutcome::Win))
                .count();
            
            patterns.push(TradingPattern {
                name: "Low Price Entry".to_string(),
                description: "Enters when market price is below 30%".to_string(),
                win_rate: wins as f64 / low_price_entries.len() as f64,
                avg_win: Decimal::ZERO, // TODO
                avg_loss: Decimal::ZERO,
                expected_value: Decimal::ZERO,
                sample_count: low_price_entries.len(),
                confidence: (low_price_entries.len() as f64 / 20.0).min(1.0),
            });
        }

        patterns
    }

    fn analyze_entries(&self, trades: &[&TradeRecord]) -> EntryInsights {
        if trades.is_empty() {
            return EntryInsights::default();
        }

        let prices: Vec<Decimal> = trades.iter().map(|t| t.entry_price).collect();
        let min_price = prices.iter().min().copied().unwrap_or(Decimal::ZERO);
        let max_price = prices.iter().max().copied().unwrap_or(Decimal::ONE);

        EntryInsights {
            preferred_price_range: (min_price, max_price),
            liquidity_preference: LiquidityPreference::NoPreference,
            avg_market_age_hours: 0.0,
            event_timing: false,
        }
    }

    fn analyze_exits(&self, _trades: &[&TradeRecord]) -> ExitInsights {
        ExitInsights::default()
    }

    /// Generate actionable recommendations
    pub fn generate_recommendations(&self, insights: &TraderInsights) -> Vec<String> {
        let mut recs = Vec::new();

        if insights.win_rate > 0.6 {
            recs.push(format!(
                "✅ High win rate ({:.0}%) - consider higher copy ratio",
                insights.win_rate * 100.0
            ));
        }

        if insights.entry_insights.preferred_price_range.0 < Decimal::new(30, 2) {
            recs.push("📊 Prefers low-priced markets (<30%) - contrarian strategy".to_string());
        }

        if insights.avg_position_size > Decimal::new(1000, 0) {
            recs.push("💰 Large position sizes - confident trader".to_string());
        }

        for pattern in &insights.patterns {
            if pattern.win_rate > 0.65 && pattern.confidence > 0.5 {
                recs.push(format!(
                    "🎯 Pattern '{}': {:.0}% win rate - apply this",
                    pattern.name, pattern.win_rate * 100.0
                ));
            }
        }

        recs
    }
}
