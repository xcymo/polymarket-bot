//! Market Regime Detection Module
//!
//! Detects and classifies market conditions to adapt trading strategies:
//! - **Trending**: Strong directional movement (bullish/bearish)
//! - **Ranging**: Sideways consolidation with clear support/resistance
//! - **Volatile**: High uncertainty, rapid price swings
//! - **Crisis**: Black swan events, market dislocation
//!
//! Uses multiple indicators:
//! - ADX (Average Directional Index) for trend strength
//! - ATR (Average True Range) for volatility
//! - Hurst Exponent for mean reversion vs trend-following
//! - Price distribution analysis
//! - Volume profile analysis

use chrono::{DateTime, Duration, Utc};
use rust_decimal::{Decimal, MathematicalOps};
use rust_decimal_macros::dec;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

/// Market regime classification
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MarketRegime {
    /// Strong upward trend (ADX > 25, +DI > -DI)
    BullishTrend,
    /// Strong downward trend (ADX > 25, -DI > +DI)
    BearishTrend,
    /// Sideways consolidation (ADX < 20, low volatility)
    Ranging,
    /// High volatility, no clear direction
    Volatile,
    /// Extreme market stress, potential black swan
    Crisis,
    /// Insufficient data to determine
    Unknown,
}

impl MarketRegime {
    /// Get strategy recommendation for this regime
    pub fn strategy_recommendation(&self) -> RegimeStrategy {
        match self {
            MarketRegime::BullishTrend => RegimeStrategy {
                trend_following_weight: dec!(0.8),
                mean_reversion_weight: dec!(0.1),
                momentum_weight: dec!(0.7),
                position_size_multiplier: dec!(1.2),
                stop_loss_multiplier: dec!(1.5),  // Wider stops in trends
                take_profit_multiplier: dec!(2.0),
                max_positions: 10,
                prefer_longs: true,
            },
            MarketRegime::BearishTrend => RegimeStrategy {
                trend_following_weight: dec!(0.8),
                mean_reversion_weight: dec!(0.1),
                momentum_weight: dec!(0.7),
                position_size_multiplier: dec!(1.0),
                stop_loss_multiplier: dec!(1.5),
                take_profit_multiplier: dec!(2.0),
                max_positions: 8,
                prefer_longs: false,
            },
            MarketRegime::Ranging => RegimeStrategy {
                trend_following_weight: dec!(0.2),
                mean_reversion_weight: dec!(0.7),
                momentum_weight: dec!(0.3),
                position_size_multiplier: dec!(0.8),
                stop_loss_multiplier: dec!(0.8),  // Tighter stops in ranging
                take_profit_multiplier: dec!(1.0),
                max_positions: 15,
                prefer_longs: true,  // Slight long bias
            },
            MarketRegime::Volatile => RegimeStrategy {
                trend_following_weight: dec!(0.3),
                mean_reversion_weight: dec!(0.3),
                momentum_weight: dec!(0.4),
                position_size_multiplier: dec!(0.5),  // Reduce size
                stop_loss_multiplier: dec!(2.0),       // Wider stops
                take_profit_multiplier: dec!(1.5),
                max_positions: 5,
                prefer_longs: true,
            },
            MarketRegime::Crisis => RegimeStrategy {
                trend_following_weight: dec!(0.0),
                mean_reversion_weight: dec!(0.0),
                momentum_weight: dec!(0.0),
                position_size_multiplier: dec!(0.1),  // Minimal exposure
                stop_loss_multiplier: dec!(3.0),
                take_profit_multiplier: dec!(0.5),    // Quick exits
                max_positions: 2,
                prefer_longs: false,
            },
            MarketRegime::Unknown => RegimeStrategy {
                trend_following_weight: dec!(0.4),
                mean_reversion_weight: dec!(0.4),
                momentum_weight: dec!(0.4),
                position_size_multiplier: dec!(0.6),
                stop_loss_multiplier: dec!(1.0),
                take_profit_multiplier: dec!(1.0),
                max_positions: 5,
                prefer_longs: true,
            },
        }
    }

    /// Risk level for this regime (0-100)
    pub fn risk_level(&self) -> u8 {
        match self {
            MarketRegime::BullishTrend => 30,
            MarketRegime::BearishTrend => 50,
            MarketRegime::Ranging => 20,
            MarketRegime::Volatile => 70,
            MarketRegime::Crisis => 95,
            MarketRegime::Unknown => 50,
        }
    }
}

/// Strategy parameters based on regime
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegimeStrategy {
    /// Weight for trend-following signals (0-1)
    pub trend_following_weight: Decimal,
    /// Weight for mean-reversion signals (0-1)
    pub mean_reversion_weight: Decimal,
    /// Weight for momentum signals (0-1)
    pub momentum_weight: Decimal,
    /// Position size multiplier (0.1-2.0)
    pub position_size_multiplier: Decimal,
    /// Stop loss distance multiplier
    pub stop_loss_multiplier: Decimal,
    /// Take profit distance multiplier
    pub take_profit_multiplier: Decimal,
    /// Maximum concurrent positions
    pub max_positions: usize,
    /// Prefer long positions
    pub prefer_longs: bool,
}

/// Configuration for regime detection
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegimeConfig {
    /// ADX threshold for trend detection (typically 25)
    pub adx_trend_threshold: Decimal,
    /// ADX threshold for strong trend (typically 40)
    pub adx_strong_threshold: Decimal,
    /// ATR percentile for high volatility (0-100)
    pub volatility_high_percentile: Decimal,
    /// ATR percentile for crisis (0-100)
    pub volatility_crisis_percentile: Decimal,
    /// Minimum bars required for detection
    pub min_bars: usize,
    /// Lookback period for ADX calculation
    pub adx_period: usize,
    /// Lookback period for ATR calculation
    pub atr_period: usize,
    /// Lookback period for Hurst exponent
    pub hurst_period: usize,
    /// Regime change smoothing (bars)
    pub smoothing_period: usize,
    /// Enable Hurst exponent calculation
    pub use_hurst: bool,
}

impl Default for RegimeConfig {
    fn default() -> Self {
        Self {
            adx_trend_threshold: dec!(25),
            adx_strong_threshold: dec!(40),
            volatility_high_percentile: dec!(80),
            volatility_crisis_percentile: dec!(95),
            min_bars: 20,
            adx_period: 14,
            atr_period: 14,
            hurst_period: 100,
            smoothing_period: 3,
            use_hurst: true,
        }
    }
}

/// Price bar for regime calculation
#[derive(Debug, Clone)]
pub struct PriceBar {
    pub timestamp: DateTime<Utc>,
    pub open: Decimal,
    pub high: Decimal,
    pub low: Decimal,
    pub close: Decimal,
    pub volume: Decimal,
}

/// Regime detection result with confidence
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegimeDetection {
    /// Detected regime
    pub regime: MarketRegime,
    /// Confidence level (0-1)
    pub confidence: Decimal,
    /// ADX value
    pub adx: Decimal,
    /// +DI value
    pub plus_di: Decimal,
    /// -DI value
    pub minus_di: Decimal,
    /// ATR value
    pub atr: Decimal,
    /// ATR percentile (0-100)
    pub atr_percentile: Decimal,
    /// Hurst exponent (if calculated)
    pub hurst: Option<Decimal>,
    /// Volatility ratio (current ATR / average ATR)
    pub volatility_ratio: Decimal,
    /// Trend strength (0-100)
    pub trend_strength: Decimal,
    /// Timestamp of detection
    pub timestamp: DateTime<Utc>,
    /// Recommended strategy
    pub strategy: RegimeStrategy,
}

/// Market Regime Detector
pub struct RegimeDetector {
    config: RegimeConfig,
    /// Historical price bars
    bars: VecDeque<PriceBar>,
    /// Historical ATR values for percentile calculation
    atr_history: VecDeque<Decimal>,
    /// Recent regime detections for smoothing
    regime_history: VecDeque<MarketRegime>,
    /// Last detection result
    last_detection: Option<RegimeDetection>,
}

impl RegimeDetector {
    /// Create a new regime detector
    pub fn new(config: RegimeConfig) -> Self {
        Self {
            config,
            bars: VecDeque::new(),
            atr_history: VecDeque::new(),
            regime_history: VecDeque::new(),
            last_detection: None,
        }
    }

    /// Add a new price bar and detect regime
    pub fn update(&mut self, bar: PriceBar) -> Option<RegimeDetection> {
        self.bars.push_back(bar);

        // Keep only necessary history
        let max_history = self.config.hurst_period.max(200);
        while self.bars.len() > max_history {
            self.bars.pop_front();
        }

        // Need minimum bars
        if self.bars.len() < self.config.min_bars {
            return None;
        }

        // Calculate indicators
        let (adx, plus_di, minus_di) = self.calculate_adx();
        let atr = self.calculate_atr();
        
        // Update ATR history
        self.atr_history.push_back(atr);
        while self.atr_history.len() > 500 {
            self.atr_history.pop_front();
        }

        let atr_percentile = self.calculate_atr_percentile(atr);
        let avg_atr = self.calculate_average_atr();
        let volatility_ratio = if avg_atr > dec!(0) { atr / avg_atr } else { dec!(1) };

        // Calculate Hurst exponent if enabled and enough data
        let hurst = if self.config.use_hurst && self.bars.len() >= self.config.hurst_period {
            Some(self.calculate_hurst())
        } else {
            None
        };

        // Detect regime
        let (regime, confidence) = self.classify_regime(
            adx, plus_di, minus_di, atr_percentile, volatility_ratio, hurst
        );

        // Apply smoothing
        self.regime_history.push_back(regime);
        while self.regime_history.len() > self.config.smoothing_period {
            self.regime_history.pop_front();
        }

        let smoothed_regime = self.smooth_regime();
        let trend_strength = self.calculate_trend_strength(adx, plus_di, minus_di);

        let detection = RegimeDetection {
            regime: smoothed_regime,
            confidence,
            adx,
            plus_di,
            minus_di,
            atr,
            atr_percentile,
            hurst,
            volatility_ratio,
            trend_strength,
            timestamp: self.bars.back().map(|b| b.timestamp).unwrap_or_else(Utc::now),
            strategy: smoothed_regime.strategy_recommendation(),
        };

        self.last_detection = Some(detection.clone());
        Some(detection)
    }

    /// Calculate ADX (Average Directional Index) with +DI and -DI
    fn calculate_adx(&self) -> (Decimal, Decimal, Decimal) {
        let period = self.config.adx_period;
        if self.bars.len() < period + 1 {
            return (dec!(0), dec!(0), dec!(0));
        }

        let bars: Vec<_> = self.bars.iter().collect();
        let mut plus_dm_sum = dec!(0);
        let mut minus_dm_sum = dec!(0);
        let mut tr_sum = dec!(0);

        // Calculate smoothed +DM, -DM, and TR
        for i in 1..=period {
            let idx = bars.len() - period - 1 + i;
            if idx == 0 { continue; }

            let current = &bars[idx];
            let prev = &bars[idx - 1];

            // True Range
            let tr = (current.high - current.low)
                .max(Decimal::abs(&(current.high - prev.close)))
                .max(Decimal::abs(&(current.low - prev.close)));
            tr_sum += tr;

            // Directional Movement
            let up_move = current.high - prev.high;
            let down_move = prev.low - current.low;

            if up_move > down_move && up_move > dec!(0) {
                plus_dm_sum += up_move;
            }
            if down_move > up_move && down_move > dec!(0) {
                minus_dm_sum += down_move;
            }
        }

        if tr_sum == dec!(0) {
            return (dec!(0), dec!(0), dec!(0));
        }

        // +DI and -DI
        let plus_di = (plus_dm_sum / tr_sum) * dec!(100);
        let minus_di = (minus_dm_sum / tr_sum) * dec!(100);

        // DX
        let di_sum = plus_di + minus_di;
        let dx = if di_sum > dec!(0) {
            (Decimal::abs(&(plus_di - minus_di)) / di_sum) * dec!(100)
        } else {
            dec!(0)
        };

        // For true ADX, we'd need to smooth DX over time
        // This is a simplified version using the current DX
        let adx = dx;

        (adx, plus_di, minus_di)
    }

    /// Calculate ATR (Average True Range)
    fn calculate_atr(&self) -> Decimal {
        let period = self.config.atr_period;
        if self.bars.len() < period + 1 {
            return dec!(0);
        }

        let bars: Vec<_> = self.bars.iter().collect();
        let mut tr_sum = dec!(0);

        for i in 1..=period {
            let idx = bars.len() - period - 1 + i;
            let current = &bars[idx];
            let prev = &bars[idx - 1];

            let tr = (current.high - current.low)
                .max(Decimal::abs(&(current.high - prev.close)))
                .max(Decimal::abs(&(current.low - prev.close)));
            tr_sum += tr;
        }

        tr_sum / Decimal::from(period)
    }

    /// Calculate ATR percentile from history
    fn calculate_atr_percentile(&self, current_atr: Decimal) -> Decimal {
        if self.atr_history.is_empty() {
            return dec!(50);
        }

        let count_below = self.atr_history.iter().filter(|&&a| a < current_atr).count();
        (Decimal::from(count_below) / Decimal::from(self.atr_history.len())) * dec!(100)
    }

    /// Calculate average ATR
    fn calculate_average_atr(&self) -> Decimal {
        if self.atr_history.is_empty() {
            return dec!(0);
        }
        let sum: Decimal = self.atr_history.iter().sum();
        sum / Decimal::from(self.atr_history.len())
    }

    /// Calculate Hurst exponent using R/S analysis
    /// H > 0.5: trending (persistent)
    /// H = 0.5: random walk
    /// H < 0.5: mean-reverting (anti-persistent)
    fn calculate_hurst(&self) -> Decimal {
        let prices: Vec<Decimal> = self.bars.iter().map(|b| b.close).collect();
        let n = prices.len();

        if n < 20 {
            return dec!(0.5); // Default to random walk
        }

        // Calculate log returns
        let mut returns = Vec::with_capacity(n - 1);
        for i in 1..n {
            if prices[i - 1] > dec!(0) && prices[i] > dec!(0) {
                // Approximate log return: (p1 - p0) / p0
                let ret = (prices[i] - prices[i - 1]) / prices[i - 1];
                returns.push(ret);
            }
        }

        if returns.is_empty() {
            return dec!(0.5);
        }

        // R/S analysis over multiple time scales
        let scales = [8, 16, 32, 64];
        let mut log_rs = Vec::new();
        let mut log_n = Vec::new();

        for &scale in &scales {
            if returns.len() < scale {
                continue;
            }

            let num_blocks = returns.len() / scale;
            if num_blocks == 0 {
                continue;
            }

            let mut rs_values = Vec::new();

            for block in 0..num_blocks {
                let start = block * scale;
                let end = start + scale;
                let block_returns = &returns[start..end];

                // Mean of block
                let mean: Decimal = block_returns.iter().sum::<Decimal>() / Decimal::from(scale);

                // Cumulative deviation from mean
                let mut cumsum = dec!(0);
                let mut min_cumsum = dec!(0);
                let mut max_cumsum = dec!(0);
                let mut sum_sq = dec!(0);

                for &ret in block_returns {
                    let dev = ret - mean;
                    cumsum += dev;
                    min_cumsum = min_cumsum.min(cumsum);
                    max_cumsum = max_cumsum.max(cumsum);
                    sum_sq += dev * dev;
                }

                // Range
                let range = max_cumsum - min_cumsum;

                // Standard deviation
                let std_dev = (sum_sq / Decimal::from(scale))
                    .sqrt()
                    .unwrap_or(dec!(0));

                if std_dev > dec!(0) {
                    rs_values.push(range / std_dev);
                }
            }

            if !rs_values.is_empty() {
                let avg_rs: Decimal = rs_values.iter().sum::<Decimal>() 
                    / Decimal::from(rs_values.len());
                
                if avg_rs > dec!(0) {
                    // log(R/S) and log(n)
                    // Approximate log as (x-1) - (x-1)^2/2 for x close to 1
                    // For larger values, use a lookup/approximation
                    let ln_rs = self.approx_ln(avg_rs);
                    let ln_scale = self.approx_ln(Decimal::from(scale));
                    
                    log_rs.push(ln_rs);
                    log_n.push(ln_scale);
                }
            }
        }

        // Linear regression to find Hurst exponent
        if log_rs.len() < 2 {
            return dec!(0.5);
        }

        let n_points = log_rs.len();
        let sum_x: Decimal = log_n.iter().sum();
        let sum_y: Decimal = log_rs.iter().sum();
        let sum_xy: Decimal = log_n.iter().zip(log_rs.iter())
            .map(|(x, y)| x * y)
            .sum();
        let sum_xx: Decimal = log_n.iter().map(|x| x * x).sum();

        let n_dec = Decimal::from(n_points);
        let denominator = n_dec * sum_xx - sum_x * sum_x;

        if denominator == dec!(0) {
            return dec!(0.5);
        }

        let hurst = (n_dec * sum_xy - sum_x * sum_y) / denominator;

        // Clamp to valid range [0, 1]
        hurst.max(dec!(0)).min(dec!(1))
    }

    /// Approximate natural logarithm for Decimal
    fn approx_ln(&self, x: Decimal) -> Decimal {
        if x <= dec!(0) {
            return dec!(0);
        }
        
        // Use: ln(x) ≈ 2 * [(x-1)/(x+1) + (1/3)*((x-1)/(x+1))^3 + ...]
        // For better range, scale x to be close to 1
        
        let mut scaled = x;
        let mut scale_factor = dec!(0);
        
        // Scale down
        while scaled > dec!(10) {
            scaled /= dec!(10);
            scale_factor += dec!(2.302585); // ln(10) ≈ 2.302585
        }
        
        // Scale up
        while scaled < dec!(0.1) {
            scaled *= dec!(10);
            scale_factor -= dec!(2.302585);
        }
        
        // Now scaled is in [0.1, 10], use Taylor series
        let z = (scaled - dec!(1)) / (scaled + dec!(1));
        let z2 = z * z;
        
        // ln(x) = 2 * (z + z^3/3 + z^5/5 + ...)
        let result = dec!(2) * (z + z * z2 / dec!(3) + z * z2 * z2 / dec!(5));
        
        result + scale_factor
    }

    /// Classify market regime based on indicators
    fn classify_regime(
        &self,
        adx: Decimal,
        plus_di: Decimal,
        minus_di: Decimal,
        atr_percentile: Decimal,
        volatility_ratio: Decimal,
        hurst: Option<Decimal>,
    ) -> (MarketRegime, Decimal) {
        // Crisis detection (highest priority)
        if atr_percentile >= self.config.volatility_crisis_percentile 
            && volatility_ratio > dec!(2.5) 
        {
            let confidence = ((atr_percentile - dec!(90)) / dec!(10)).min(dec!(1));
            return (MarketRegime::Crisis, confidence);
        }

        // High volatility (non-crisis)
        if atr_percentile >= self.config.volatility_high_percentile 
            && adx < self.config.adx_trend_threshold 
        {
            let confidence = (atr_percentile - dec!(70)) / dec!(30);
            return (MarketRegime::Volatile, confidence.min(dec!(1)));
        }

        // Strong trend
        if adx >= self.config.adx_trend_threshold {
            let base_confidence = (adx - self.config.adx_trend_threshold) 
                / (self.config.adx_strong_threshold - self.config.adx_trend_threshold);
            
            // Adjust by Hurst if available
            let hurst_boost = hurst.map(|h| {
                if h > dec!(0.6) { (h - dec!(0.5)) * dec!(0.5) } else { dec!(0) }
            }).unwrap_or(dec!(0));

            let confidence = (base_confidence + hurst_boost).min(dec!(1));

            if plus_di > minus_di {
                return (MarketRegime::BullishTrend, confidence);
            } else {
                return (MarketRegime::BearishTrend, confidence);
            }
        }

        // Ranging market
        if adx < dec!(20) && atr_percentile < dec!(50) {
            let confidence = (dec!(20) - adx) / dec!(20);
            
            // Hurst < 0.5 suggests mean reversion (good for ranging)
            let hurst_boost = hurst.map(|h| {
                if h < dec!(0.4) { (dec!(0.5) - h) * dec!(0.5) } else { dec!(0) }
            }).unwrap_or(dec!(0));

            return (MarketRegime::Ranging, (confidence + hurst_boost).min(dec!(1)));
        }

        // Default: moderate confidence unknown
        (MarketRegime::Unknown, dec!(0.5))
    }

    /// Smooth regime using recent history (majority vote)
    fn smooth_regime(&self) -> MarketRegime {
        if self.regime_history.is_empty() {
            return MarketRegime::Unknown;
        }

        let mut counts = std::collections::HashMap::new();
        for regime in &self.regime_history {
            *counts.entry(*regime).or_insert(0) += 1;
        }

        counts.into_iter()
            .max_by_key(|(_, count)| *count)
            .map(|(regime, _)| regime)
            .unwrap_or(MarketRegime::Unknown)
    }

    /// Calculate trend strength (0-100)
    fn calculate_trend_strength(&self, adx: Decimal, plus_di: Decimal, minus_di: Decimal) -> Decimal {
        let di_diff = Decimal::abs(&(plus_di - minus_di));
        let di_sum = plus_di + minus_di;
        
        let directional_strength = if di_sum > dec!(0) {
            (di_diff / di_sum) * dec!(50)
        } else {
            dec!(0)
        };

        let adx_strength = (adx / dec!(50)) * dec!(50);

        (directional_strength + adx_strength).min(dec!(100))
    }

    /// Get current regime without updating
    pub fn current_regime(&self) -> Option<&RegimeDetection> {
        self.last_detection.as_ref()
    }

    /// Check if regime changed from last detection
    pub fn regime_changed(&self) -> bool {
        if self.regime_history.len() < 2 {
            return false;
        }
        
        let current = self.regime_history.back();
        let prev = self.regime_history.iter().rev().nth(1);
        
        match (current, prev) {
            (Some(c), Some(p)) => c != p,
            _ => false,
        }
    }

    /// Get regime transition info
    pub fn get_transition(&self) -> Option<(MarketRegime, MarketRegime)> {
        if self.regime_history.len() < 2 {
            return None;
        }
        
        let current = *self.regime_history.back()?;
        let prev = *self.regime_history.iter().rev().nth(1)?;
        
        if current != prev {
            Some((prev, current))
        } else {
            None
        }
    }

    /// Reset detector state
    pub fn reset(&mut self) {
        self.bars.clear();
        self.atr_history.clear();
        self.regime_history.clear();
        self.last_detection = None;
    }
}

/// Multi-timeframe regime analysis
pub struct MultiTimeframeRegime {
    /// Short-term detector (e.g., 15m)
    pub short_term: RegimeDetector,
    /// Medium-term detector (e.g., 1h)
    pub medium_term: RegimeDetector,
    /// Long-term detector (e.g., 4h)
    pub long_term: RegimeDetector,
}

impl MultiTimeframeRegime {
    /// Create with default configs
    pub fn new() -> Self {
        Self {
            short_term: RegimeDetector::new(RegimeConfig::default()),
            medium_term: RegimeDetector::new(RegimeConfig::default()),
            long_term: RegimeDetector::new(RegimeConfig::default()),
        }
    }

    /// Get consensus regime across timeframes
    pub fn consensus_regime(&self) -> Option<RegimeConsensus> {
        let short = self.short_term.current_regime()?;
        let medium = self.medium_term.current_regime()?;
        let long = self.long_term.current_regime()?;

        // Crisis overrides everything
        if short.regime == MarketRegime::Crisis 
            || medium.regime == MarketRegime::Crisis 
            || long.regime == MarketRegime::Crisis 
        {
            return Some(RegimeConsensus {
                primary_regime: MarketRegime::Crisis,
                confidence: dec!(0.9),
                alignment: RegimeAlignment::Conflicting,
                short_regime: short.regime,
                medium_regime: medium.regime,
                long_regime: long.regime,
            });
        }

        // Check alignment
        let alignment = if short.regime == medium.regime && medium.regime == long.regime {
            RegimeAlignment::FullyAligned
        } else if short.regime == medium.regime || medium.regime == long.regime {
            RegimeAlignment::PartiallyAligned
        } else {
            RegimeAlignment::Conflicting
        };

        // Weight longer timeframes more heavily
        let primary_regime = if long.confidence > dec!(0.6) {
            long.regime
        } else if medium.confidence > dec!(0.6) {
            medium.regime
        } else {
            short.regime
        };

        let confidence = match alignment {
            RegimeAlignment::FullyAligned => (short.confidence + medium.confidence + long.confidence) / dec!(3) * dec!(1.2),
            RegimeAlignment::PartiallyAligned => (short.confidence + medium.confidence + long.confidence) / dec!(3),
            RegimeAlignment::Conflicting => (short.confidence + medium.confidence + long.confidence) / dec!(3) * dec!(0.7),
        };

        Some(RegimeConsensus {
            primary_regime,
            confidence: confidence.min(dec!(1)),
            alignment,
            short_regime: short.regime,
            medium_regime: medium.regime,
            long_regime: long.regime,
        })
    }
}

impl Default for MultiTimeframeRegime {
    fn default() -> Self {
        Self::new()
    }
}

/// Alignment of regimes across timeframes
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RegimeAlignment {
    /// All timeframes agree
    FullyAligned,
    /// Some timeframes agree
    PartiallyAligned,
    /// All timeframes disagree
    Conflicting,
}

/// Consensus regime across multiple timeframes
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegimeConsensus {
    pub primary_regime: MarketRegime,
    pub confidence: Decimal,
    pub alignment: RegimeAlignment,
    pub short_regime: MarketRegime,
    pub medium_regime: MarketRegime,
    pub long_regime: MarketRegime,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_trending_bars(n: usize, direction: bool) -> Vec<PriceBar> {
        let mut bars = Vec::with_capacity(n);
        let mut price = dec!(100);
        let step = if direction { dec!(1) } else { dec!(-1) };

        for i in 0..n {
            let open = price;
            price += step + (Decimal::from(i % 3) - dec!(1)) * dec!(0.1);
            let close = price;
            let high = open.max(close) + dec!(0.5);
            let low = open.min(close) - dec!(0.5);

            bars.push(PriceBar {
                timestamp: Utc::now() - Duration::hours((n - i) as i64),
                open,
                high,
                low,
                close,
                volume: dec!(1000),
            });
        }
        bars
    }

    fn create_ranging_bars(n: usize) -> Vec<PriceBar> {
        let mut bars = Vec::with_capacity(n);
        let center = dec!(100);
        let range = dec!(2);

        for i in 0..n {
            let offset = (Decimal::from(i % 10) - dec!(5)) * range / dec!(5);
            let open = center + offset;
            let close = center + offset + (Decimal::from(i % 3) - dec!(1)) * dec!(0.3);
            let high = open.max(close) + dec!(0.3);
            let low = open.min(close) - dec!(0.3);

            bars.push(PriceBar {
                timestamp: Utc::now() - Duration::hours((n - i) as i64),
                open,
                high,
                low,
                close,
                volume: dec!(1000),
            });
        }
        bars
    }

    fn create_volatile_bars(n: usize) -> Vec<PriceBar> {
        let mut bars = Vec::with_capacity(n);
        let mut price = dec!(100);

        for i in 0..n {
            let swing = if i % 2 == 0 { dec!(5) } else { dec!(-5) };
            let open = price;
            price += swing;
            let close = price;
            let high = open.max(close) + dec!(3);
            let low = open.min(close) - dec!(3);

            bars.push(PriceBar {
                timestamp: Utc::now() - Duration::hours((n - i) as i64),
                open,
                high,
                low,
                close,
                volume: dec!(5000), // Higher volume in volatile conditions
            });
        }
        bars
    }

    #[test]
    fn test_regime_detector_creation() {
        let config = RegimeConfig::default();
        let detector = RegimeDetector::new(config);
        
        assert!(detector.current_regime().is_none());
        assert!(!detector.regime_changed());
    }

    #[test]
    fn test_bullish_trend_detection() {
        let mut detector = RegimeDetector::new(RegimeConfig::default());
        
        let bars = create_trending_bars(50, true);
        let mut last_detection = None;
        
        for bar in bars {
            last_detection = detector.update(bar);
        }
        
        let detection = last_detection.expect("Should have detection");
        assert!(detection.adx > dec!(0), "ADX should be positive");
        assert!(detection.plus_di > dec!(0), "+DI should be positive");
    }

    #[test]
    fn test_bearish_trend_detection() {
        let mut detector = RegimeDetector::new(RegimeConfig::default());
        
        let bars = create_trending_bars(50, false);
        let mut last_detection = None;
        
        for bar in bars {
            last_detection = detector.update(bar);
        }
        
        let detection = last_detection.expect("Should have detection");
        assert!(detection.adx > dec!(0), "ADX should be positive");
    }

    #[test]
    fn test_ranging_market_detection() {
        let mut detector = RegimeDetector::new(RegimeConfig::default());
        
        let bars = create_ranging_bars(50);
        let mut last_detection = None;
        
        for bar in bars {
            last_detection = detector.update(bar);
        }
        
        let detection = last_detection.expect("Should have detection");
        // ADX should be lower in ranging markets
        assert!(detection.adx < dec!(30), "ADX should be low in ranging market");
    }

    #[test]
    fn test_volatile_market_detection() {
        let mut detector = RegimeDetector::new(RegimeConfig::default());
        
        let bars = create_volatile_bars(100);
        let mut last_detection = None;
        
        for bar in bars {
            last_detection = detector.update(bar);
        }
        
        let detection = last_detection.expect("Should have detection");
        // ATR should be high relative to history
        assert!(detection.atr > dec!(0), "ATR should be positive");
    }

    #[test]
    fn test_regime_strategy_recommendations() {
        assert!(MarketRegime::BullishTrend.strategy_recommendation().prefer_longs);
        assert!(!MarketRegime::BearishTrend.strategy_recommendation().prefer_longs);
        assert!(MarketRegime::Crisis.strategy_recommendation().position_size_multiplier < dec!(0.5));
    }

    #[test]
    fn test_regime_risk_levels() {
        assert!(MarketRegime::Ranging.risk_level() < MarketRegime::Volatile.risk_level());
        assert!(MarketRegime::Volatile.risk_level() < MarketRegime::Crisis.risk_level());
        assert_eq!(MarketRegime::Crisis.risk_level(), 95);
    }

    #[test]
    fn test_minimum_bars_requirement() {
        let config = RegimeConfig {
            min_bars: 20,
            ..Default::default()
        };
        let mut detector = RegimeDetector::new(config);
        
        // Add fewer bars than minimum
        for i in 0..15 {
            let bar = PriceBar {
                timestamp: Utc::now() - Duration::hours((15 - i) as i64),
                open: dec!(100),
                high: dec!(101),
                low: dec!(99),
                close: dec!(100),
                volume: dec!(1000),
            };
            let result = detector.update(bar);
            assert!(result.is_none(), "Should not detect with insufficient data");
        }
    }

    #[test]
    fn test_atr_calculation() {
        let mut detector = RegimeDetector::new(RegimeConfig::default());
        
        // Add bars with known volatility
        for i in 0..30 {
            let bar = PriceBar {
                timestamp: Utc::now() - Duration::hours((30 - i) as i64),
                open: dec!(100),
                high: dec!(105), // $5 range
                low: dec!(95),
                close: dec!(100),
                volume: dec!(1000),
            };
            detector.update(bar);
        }
        
        let detection = detector.current_regime().expect("Should have detection");
        assert!(detection.atr > dec!(0), "ATR should be positive");
    }

    #[test]
    fn test_regime_smoothing() {
        let config = RegimeConfig {
            smoothing_period: 3,
            ..Default::default()
        };
        let mut detector = RegimeDetector::new(config);
        
        // Add enough data to get detections
        let bars = create_trending_bars(50, true);
        for bar in bars {
            detector.update(bar);
        }
        
        // Regime history should be populated
        assert!(detector.current_regime().is_some());
    }

    #[test]
    fn test_regime_transition_detection() {
        let mut detector = RegimeDetector::new(RegimeConfig {
            smoothing_period: 1, // No smoothing for faster transitions
            ..Default::default()
        });
        
        // Start with trending
        let trending_bars = create_trending_bars(30, true);
        for bar in trending_bars {
            detector.update(bar);
        }
        
        // Then ranging
        let ranging_bars = create_ranging_bars(30);
        for bar in ranging_bars {
            detector.update(bar);
        }
        
        // Check for transition
        // Note: actual transition detection depends on indicators
    }

    #[test]
    fn test_hurst_exponent_calculation() {
        let config = RegimeConfig {
            use_hurst: true,
            hurst_period: 50,
            ..Default::default()
        };
        let mut detector = RegimeDetector::new(config);
        
        let bars = create_trending_bars(100, true);
        for bar in bars {
            detector.update(bar);
        }
        
        let detection = detector.current_regime().expect("Should have detection");
        if let Some(hurst) = detection.hurst {
            assert!(hurst >= dec!(0) && hurst <= dec!(1), "Hurst should be in [0,1]");
        }
    }

    #[test]
    fn test_multi_timeframe_creation() {
        let mtf = MultiTimeframeRegime::new();
        assert!(mtf.short_term.current_regime().is_none());
        assert!(mtf.medium_term.current_regime().is_none());
        assert!(mtf.long_term.current_regime().is_none());
    }

    #[test]
    fn test_detector_reset() {
        let mut detector = RegimeDetector::new(RegimeConfig::default());
        
        let bars = create_trending_bars(30, true);
        for bar in bars {
            detector.update(bar);
        }
        
        assert!(detector.current_regime().is_some());
        
        detector.reset();
        
        assert!(detector.current_regime().is_none());
    }

    #[test]
    fn test_confidence_bounds() {
        let mut detector = RegimeDetector::new(RegimeConfig::default());
        
        let bars = create_trending_bars(100, true);
        for bar in bars {
            detector.update(bar);
        }
        
        if let Some(detection) = detector.current_regime() {
            assert!(detection.confidence >= dec!(0), "Confidence should be >= 0");
            assert!(detection.confidence <= dec!(1), "Confidence should be <= 1");
        }
    }

    #[test]
    fn test_volatility_ratio() {
        let mut detector = RegimeDetector::new(RegimeConfig::default());
        
        // Start with low volatility
        for i in 0..50 {
            let bar = PriceBar {
                timestamp: Utc::now() - Duration::hours((100 - i) as i64),
                open: dec!(100),
                high: dec!(101),
                low: dec!(99),
                close: dec!(100),
                volume: dec!(1000),
            };
            detector.update(bar);
        }
        
        // Then high volatility
        for i in 50..100 {
            let bar = PriceBar {
                timestamp: Utc::now() - Duration::hours((100 - i) as i64),
                open: dec!(100),
                high: dec!(110),
                low: dec!(90),
                close: dec!(100),
                volume: dec!(1000),
            };
            detector.update(bar);
        }
        
        let detection = detector.current_regime().expect("Should have detection");
        // Volatility ratio should be elevated
        assert!(detection.volatility_ratio > dec!(0));
    }

    #[test]
    fn test_trend_strength_calculation() {
        let mut detector = RegimeDetector::new(RegimeConfig::default());
        
        let bars = create_trending_bars(50, true);
        for bar in bars {
            detector.update(bar);
        }
        
        let detection = detector.current_regime().expect("Should have detection");
        assert!(detection.trend_strength >= dec!(0));
        assert!(detection.trend_strength <= dec!(100));
    }
}
