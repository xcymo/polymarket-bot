//! Pattern recognition for trading strategies

use super::{TradeRecord, TradeOutcome, TradingPattern};
use rust_decimal::Decimal;

/// Detect common trading patterns in a trade history
pub fn detect_all_patterns(trades: &[TradeRecord]) -> Vec<TradingPattern> {
    let mut patterns = Vec::new();

    // 1. Contrarian pattern - buys low, sells high
    if let Some(p) = detect_contrarian(trades) {
        patterns.push(p);
    }

    // 2. Momentum pattern - follows trends
    if let Some(p) = detect_momentum(trades) {
        patterns.push(p);
    }

    // 3. Size scaling pattern - increases size on wins
    if let Some(p) = detect_size_scaling(trades) {
        patterns.push(p);
    }

    // 4. Quick flip pattern - short hold times
    if let Some(p) = detect_quick_flip(trades) {
        patterns.push(p);
    }

    patterns
}

fn detect_contrarian(trades: &[TradeRecord]) -> Option<TradingPattern> {
    // Contrarian: buys when price < 35% or > 65%
    let contrarian_trades: Vec<_> = trades.iter()
        .filter(|t| {
            t.entry_price < Decimal::new(35, 2) || t.entry_price > Decimal::new(65, 2)
        })
        .collect();

    if contrarian_trades.len() < 5 {
        return None;
    }

    let wins = contrarian_trades.iter()
        .filter(|t| t.outcome == Some(TradeOutcome::Win))
        .count();

    let win_rate = wins as f64 / contrarian_trades.len() as f64;

    Some(TradingPattern {
        name: "Contrarian".to_string(),
        description: "Enters positions at extreme prices (<35% or >65%)".to_string(),
        win_rate,
        avg_win: Decimal::ZERO,
        avg_loss: Decimal::ZERO,
        expected_value: Decimal::ZERO,
        sample_count: contrarian_trades.len(),
        confidence: (contrarian_trades.len() as f64 / 20.0).min(1.0),
    })
}

fn detect_momentum(_trades: &[TradeRecord]) -> Option<TradingPattern> {
    // Momentum: buys when price is moving in one direction
    // Would need price history to detect properly
    None
}

fn detect_size_scaling(trades: &[TradeRecord]) -> Option<TradingPattern> {
    // Check if trader increases size after wins
    if trades.len() < 10 {
        return None;
    }

    let mut size_after_win = Vec::new();
    let mut size_after_loss = Vec::new();

    for i in 1..trades.len() {
        match trades[i - 1].outcome {
            Some(TradeOutcome::Win) => size_after_win.push(trades[i].size),
            Some(TradeOutcome::Loss) => size_after_loss.push(trades[i].size),
            _ => {}
        }
    }

    if size_after_win.is_empty() || size_after_loss.is_empty() {
        return None;
    }

    let avg_after_win: Decimal = size_after_win.iter().sum::<Decimal>() 
        / Decimal::from(size_after_win.len());
    let avg_after_loss: Decimal = size_after_loss.iter().sum::<Decimal>() 
        / Decimal::from(size_after_loss.len());

    if avg_after_win > avg_after_loss * Decimal::new(12, 1) {
        Some(TradingPattern {
            name: "Martingale-lite".to_string(),
            description: "Increases position size after wins".to_string(),
            win_rate: 0.0,
            avg_win: avg_after_win,
            avg_loss: avg_after_loss,
            expected_value: Decimal::ZERO,
            sample_count: trades.len(),
            confidence: 0.6,
        })
    } else {
        None
    }
}

fn detect_quick_flip(trades: &[TradeRecord]) -> Option<TradingPattern> {
    // Quick flip: holds for < 24 hours
    let quick_trades: Vec<_> = trades.iter()
        .filter(|t| {
            if let (Some(exit), entry) = (t.exit_time, t.entry_time) {
                (exit - entry).num_hours() < 24
            } else {
                false
            }
        })
        .collect();

    if quick_trades.len() < 5 {
        return None;
    }

    let wins = quick_trades.iter()
        .filter(|t| t.outcome == Some(TradeOutcome::Win))
        .count();

    Some(TradingPattern {
        name: "Quick Flip".to_string(),
        description: "Exits positions within 24 hours".to_string(),
        win_rate: wins as f64 / quick_trades.len() as f64,
        avg_win: Decimal::ZERO,
        avg_loss: Decimal::ZERO,
        expected_value: Decimal::ZERO,
        sample_count: quick_trades.len(),
        confidence: (quick_trades.len() as f64 / 20.0).min(1.0),
    })
}
