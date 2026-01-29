//! Trader profiling and categorization

use super::TraderInsights;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

/// Trader profile category
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub enum TraderType {
    /// High frequency, small edge
    Scalper,
    /// Event-driven trader
    EventTrader,
    /// Follows trends
    MomentumTrader,
    /// Bets against the crowd
    Contrarian,
    /// Large, concentrated bets
    Whale,
    /// Diversified, risk-managed
    Conservative,
    /// Unknown or mixed style
    Unknown,
}

/// Profile a trader based on their history
pub fn profile_trader(insights: &TraderInsights) -> TraderProfile {
    let trader_type = determine_type(insights);
    let risk_level = calculate_risk_level(insights);
    let skill_score = calculate_skill_score(insights);

    TraderProfile {
        trader: insights.trader.clone(),
        trader_type,
        risk_level,
        skill_score,
        follow_recommendation: generate_follow_rec(&trader_type, skill_score, risk_level),
        copy_ratio_suggestion: suggest_copy_ratio(&trader_type, skill_score),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraderProfile {
    pub trader: String,
    pub trader_type: TraderType,
    pub risk_level: RiskLevel,
    pub skill_score: f64,  // 0-100
    pub follow_recommendation: FollowRecommendation,
    pub copy_ratio_suggestion: f64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub enum RiskLevel {
    Low,
    Medium,
    High,
    Extreme,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FollowRecommendation {
    StrongFollow { reason: String },
    ModerateFollow { reason: String },
    CautiousFollow { reason: String },
    DoNotFollow { reason: String },
}

fn determine_type(insights: &TraderInsights) -> TraderType {
    // Check for patterns
    let has_contrarian = insights.patterns.iter()
        .any(|p| p.name.to_lowercase().contains("contrarian"));
    
    let has_quick_flip = insights.patterns.iter()
        .any(|p| p.name.to_lowercase().contains("quick"));

    // Analyze position sizes
    let is_whale = insights.avg_position_size > Decimal::new(5000, 0);
    
    // Analyze win rate and trade count
    let is_high_frequency = insights.total_trades > 50;
    
    if has_contrarian && insights.win_rate > 0.55 {
        TraderType::Contrarian
    } else if has_quick_flip && is_high_frequency {
        TraderType::Scalper
    } else if is_whale && insights.win_rate > 0.6 {
        TraderType::Whale
    } else if insights.win_rate > 0.65 && insights.total_trades > 20 {
        TraderType::MomentumTrader
    } else if insights.total_pnl > Decimal::ZERO && insights.win_rate > 0.5 {
        TraderType::Conservative
    } else {
        TraderType::Unknown
    }
}

fn calculate_risk_level(insights: &TraderInsights) -> RiskLevel {
    // Higher avg position = higher risk
    // Lower win rate with high trades = higher risk
    
    let position_risk = if insights.avg_position_size > Decimal::new(10000, 0) {
        3.0
    } else if insights.avg_position_size > Decimal::new(5000, 0) {
        2.0
    } else if insights.avg_position_size > Decimal::new(1000, 0) {
        1.0
    } else {
        0.5
    };

    let win_rate_risk = if insights.win_rate < 0.4 {
        3.0
    } else if insights.win_rate < 0.5 {
        2.0
    } else if insights.win_rate < 0.6 {
        1.0
    } else {
        0.5
    };

    let total_risk = position_risk + win_rate_risk;

    if total_risk > 5.0 {
        RiskLevel::Extreme
    } else if total_risk > 3.5 {
        RiskLevel::High
    } else if total_risk > 2.0 {
        RiskLevel::Medium
    } else {
        RiskLevel::Low
    }
}

fn calculate_skill_score(insights: &TraderInsights) -> f64 {
    // Skill score based on:
    // - Win rate (40%)
    // - Consistency (20%)
    // - Pattern recognition quality (20%)
    // - Total profit (20%)

    let win_rate_score = insights.win_rate * 40.0;
    
    let consistency_score = if insights.total_trades > 30 && insights.win_rate > 0.55 {
        20.0
    } else if insights.total_trades > 10 {
        10.0
    } else {
        5.0
    };

    let pattern_score = insights.patterns.iter()
        .filter(|p| p.win_rate > 0.6)
        .count() as f64 * 5.0;
    let pattern_score = pattern_score.min(20.0);

    let profit_score = if insights.total_pnl > Decimal::new(10000, 0) {
        20.0
    } else if insights.total_pnl > Decimal::new(1000, 0) {
        15.0
    } else if insights.total_pnl > Decimal::ZERO {
        10.0
    } else {
        0.0
    };

    win_rate_score + consistency_score + pattern_score + profit_score
}

fn generate_follow_rec(trader_type: &TraderType, skill: f64, risk: RiskLevel) -> FollowRecommendation {
    if skill >= 70.0 && risk != RiskLevel::Extreme {
        FollowRecommendation::StrongFollow {
            reason: format!("High skill ({:.0}), manageable risk", skill),
        }
    } else if skill >= 50.0 {
        FollowRecommendation::ModerateFollow {
            reason: format!("Decent skill ({:.0}), worth watching", skill),
        }
    } else if skill >= 30.0 && *trader_type != TraderType::Unknown {
        FollowRecommendation::CautiousFollow {
            reason: format!("Some skill ({:.0}), proceed carefully", skill),
        }
    } else {
        FollowRecommendation::DoNotFollow {
            reason: format!("Low skill ({:.0}) or unknown pattern", skill),
        }
    }
}

fn suggest_copy_ratio(trader_type: &TraderType, skill: f64) -> f64 {
    // Higher skill = higher copy ratio
    // But cap based on trader type
    
    let base_ratio = (skill / 100.0).min(0.8);
    
    match trader_type {
        TraderType::Whale => base_ratio * 0.5,  // Whales take big risks, reduce
        TraderType::Scalper => base_ratio * 0.7,  // Scalpers need quick execution
        TraderType::Conservative => base_ratio * 1.0,  // Safe to follow closely
        TraderType::Contrarian => base_ratio * 0.6,  // Contrarian can be risky
        _ => base_ratio * 0.5,
    }
}
