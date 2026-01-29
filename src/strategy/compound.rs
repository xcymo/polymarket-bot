//! Compound growth strategy with dynamic position sizing
//!
//! Implements:
//! - Dynamic Kelly based on win streak
//! - Automatic compounding (reinvest profits)
//! - Volatility-adjusted sizing
//! - Progressive risk scaling

use crate::config::{RiskConfig, StrategyConfig};
use crate::model::Prediction;
use crate::types::{Market, Side, Signal};
use chrono::Utc;
use rust_decimal::{Decimal, MathematicalOps};
use rust_decimal_macros::dec;
use std::collections::VecDeque;

/// Enhanced signal generator with compound growth optimization
pub struct CompoundStrategy {
    config: StrategyConfig,
    risk_config: RiskConfig,
    
    // Performance tracking for dynamic sizing
    recent_results: VecDeque<TradeResult>,
    win_streak: u32,
    lose_streak: u32,
    
    // Compound growth state
    initial_balance: Decimal,
    peak_balance: Decimal,
    
    // Dynamic parameters
    current_kelly_mult: Decimal,  // 1.0 = base, can scale 0.5 - 2.0
}

#[derive(Debug, Clone)]
struct TradeResult {
    pnl: Decimal,
    #[allow(dead_code)]
    edge: Decimal,
    confidence: Decimal,
    #[allow(dead_code)]
    timestamp: chrono::DateTime<Utc>,
}

impl CompoundStrategy {
    pub fn new(config: StrategyConfig, risk_config: RiskConfig, initial_balance: Decimal) -> Self {
        Self {
            config,
            risk_config,
            recent_results: VecDeque::with_capacity(50),
            win_streak: 0,
            lose_streak: 0,
            initial_balance,
            peak_balance: initial_balance,
            current_kelly_mult: dec!(1.0),
        }
    }

    /// Update with trade result for dynamic adjustment
    pub fn record_result(&mut self, pnl: Decimal, edge: Decimal, confidence: Decimal) {
        let result = TradeResult {
            pnl,
            edge,
            confidence,
            timestamp: Utc::now(),
        };
        
        // Update streaks
        if pnl > Decimal::ZERO {
            self.win_streak += 1;
            self.lose_streak = 0;
        } else {
            self.lose_streak += 1;
            self.win_streak = 0;
        }
        
        // Store result
        self.recent_results.push_back(result);
        if self.recent_results.len() > 50 {
            self.recent_results.pop_front();
        }
        
        // Adjust Kelly multiplier based on performance
        self.adjust_kelly_multiplier();
    }

    /// Update current balance for compound calculations
    pub fn update_balance(&mut self, current_balance: Decimal) {
        if current_balance > self.peak_balance {
            self.peak_balance = current_balance;
        }
    }

    /// Generate signal with compound-optimized sizing
    pub fn generate(&self, market: &Market, prediction: &Prediction, current_balance: Decimal) -> Option<Signal> {
        let market_prob = market.yes_price()?;
        let model_prob = prediction.probability;
        let edge = model_prob - market_prob;

        // Dynamic edge threshold based on confidence and recent performance
        let adjusted_min_edge = self.calculate_dynamic_edge_threshold(prediction.confidence);
        
        if edge.abs() < adjusted_min_edge {
            return None;
        }

        if prediction.confidence < self.config.min_confidence {
            return None;
        }

        let yes_token = market
            .outcomes
            .iter()
            .find(|o| o.outcome.to_lowercase() == "yes")
            .map(|o| o.token_id.clone())?;

        let (side, token_id, effective_prob) = if edge > Decimal::ZERO {
            (Side::Buy, yes_token, model_prob)
        } else {
            (Side::Sell, yes_token, Decimal::ONE - model_prob)
        };

        let market_price = if edge > Decimal::ZERO {
            market_prob
        } else {
            Decimal::ONE - market_prob
        };

        // Calculate compound-optimized position size
        let suggested_size = self.calculate_compound_size(
            effective_prob,
            market_price,
            prediction.confidence,
            current_balance,
        );

        Some(Signal {
            market_id: market.id.clone(),
            token_id,
            side,
            model_probability: model_prob,
            market_probability: market_prob,
            edge,
            confidence: prediction.confidence,
            suggested_size,
            timestamp: Utc::now(),
        })
    }

    /// Dynamic edge threshold - lower when we're hot, higher when cold
    fn calculate_dynamic_edge_threshold(&self, confidence: Decimal) -> Decimal {
        let base = self.config.min_edge;
        
        // High confidence + win streak = can take smaller edges
        if self.win_streak >= 3 && confidence >= dec!(0.75) {
            return base * dec!(0.7);  // 30% lower threshold
        }
        
        // Losing streak = require bigger edges
        if self.lose_streak >= 2 {
            return base * dec!(1.3);  // 30% higher threshold
        }
        
        base
    }

    /// Compound-optimized position sizing
    fn calculate_compound_size(
        &self,
        model_prob: Decimal,
        market_price: Decimal,
        confidence: Decimal,
        current_balance: Decimal,
    ) -> Decimal {
        let edge = model_prob - market_price;
        let potential_profit = Decimal::ONE - market_price;
        
        if potential_profit <= Decimal::ZERO {
            return Decimal::ZERO;
        }

        // Base Kelly
        let full_kelly = edge / potential_profit;
        if full_kelly <= Decimal::ZERO {
            return Decimal::ZERO;
        }

        // Apply configured Kelly fraction
        let fractional_kelly = full_kelly * self.config.kelly_fraction;
        
        // Apply dynamic multiplier (0.5 - 2.0 based on performance)
        let dynamic_kelly = fractional_kelly * self.current_kelly_mult;
        
        // Adjust by confidence
        let confidence_adjusted = dynamic_kelly * confidence;
        
        // Compound growth factor: when we're up, bet proportionally more
        let growth_factor = self.calculate_growth_factor(current_balance);
        let growth_adjusted = confidence_adjusted * growth_factor;
        
        // Drawdown protection: reduce size in drawdown
        let drawdown_adjusted = self.apply_drawdown_protection(growth_adjusted, current_balance);
        
        // Final caps
        let max_size = self.calculate_dynamic_max_position(current_balance);
        drawdown_adjusted.min(max_size)
    }

    /// Adjust Kelly multiplier based on recent performance
    fn adjust_kelly_multiplier(&mut self) {
        if self.recent_results.len() < 5 {
            return;
        }

        // Calculate recent win rate
        let wins = self.recent_results.iter().filter(|r| r.pnl > Decimal::ZERO).count();
        let win_rate = Decimal::from(wins as u32) / Decimal::from(self.recent_results.len() as u32);
        
        // Calculate if our edge estimates are accurate
        let expected_win_rate: Decimal = self.recent_results
            .iter()
            .map(|r| r.confidence)
            .sum::<Decimal>() / Decimal::from(self.recent_results.len() as u32);
        
        // If we're winning more than expected, increase Kelly
        // If we're winning less, decrease Kelly
        if win_rate > expected_win_rate + dec!(0.1) {
            self.current_kelly_mult = (self.current_kelly_mult * dec!(1.1)).min(dec!(2.0));
        } else if win_rate < expected_win_rate - dec!(0.1) {
            self.current_kelly_mult = (self.current_kelly_mult * dec!(0.9)).max(dec!(0.5));
        } else {
            // Slowly revert to 1.0
            if self.current_kelly_mult > dec!(1.0) {
                self.current_kelly_mult = self.current_kelly_mult * dec!(0.98);
            } else if self.current_kelly_mult < dec!(1.0) {
                self.current_kelly_mult = self.current_kelly_mult * dec!(1.02);
            }
        }
    }

    /// Growth factor for compound betting
    /// When balance is up, we bet proportionally more
    fn calculate_growth_factor(&self, current_balance: Decimal) -> Decimal {
        let growth = current_balance / self.initial_balance;
        
        // Sqrt scaling: if we 4x, we bet 2x more (not 4x)
        // This balances growth with risk management
        let sqrt_growth = growth.sqrt().unwrap_or(dec!(1.0));
        
        // Cap at 2x initial sizing
        sqrt_growth.min(dec!(2.0))
    }

    /// Reduce position size during drawdowns
    fn apply_drawdown_protection(&self, size: Decimal, current_balance: Decimal) -> Decimal {
        let drawdown = (self.peak_balance - current_balance) / self.peak_balance;
        
        if drawdown > dec!(0.20) {
            // More than 20% drawdown: cut size in half
            return size * dec!(0.5);
        } else if drawdown > dec!(0.10) {
            // 10-20% drawdown: reduce by 25%
            return size * dec!(0.75);
        }
        
        size
    }

    /// Dynamic max position based on current state
    fn calculate_dynamic_max_position(&self, current_balance: Decimal) -> Decimal {
        let base_max = self.risk_config.max_position_pct;
        
        // When on a win streak and balance is up, allow slightly larger positions
        if self.win_streak >= 5 && current_balance > self.initial_balance * dec!(1.2) {
            return (base_max * dec!(1.25)).min(dec!(0.10));  // Max 10%
        }
        
        // When in drawdown or losing streak, reduce max
        if self.lose_streak >= 3 {
            return base_max * dec!(0.75);
        }
        
        base_max
    }

    /// Get current strategy stats
    pub fn get_stats(&self) -> CompoundStats {
        let total_trades = self.recent_results.len();
        let wins = self.recent_results.iter().filter(|r| r.pnl > Decimal::ZERO).count();
        let total_pnl: Decimal = self.recent_results.iter().map(|r| r.pnl).sum();
        
        CompoundStats {
            total_trades,
            wins,
            win_rate: if total_trades > 0 { wins as f64 / total_trades as f64 } else { 0.0 },
            total_pnl,
            current_kelly_mult: self.current_kelly_mult,
            win_streak: self.win_streak,
            lose_streak: self.lose_streak,
            growth_from_initial: self.peak_balance / self.initial_balance,
        }
    }
}

#[derive(Debug, Clone)]
pub struct CompoundStats {
    pub total_trades: usize,
    pub wins: usize,
    pub win_rate: f64,
    pub total_pnl: Decimal,
    pub current_kelly_mult: Decimal,
    pub win_streak: u32,
    pub lose_streak: u32,
    pub growth_from_initial: Decimal,
}
