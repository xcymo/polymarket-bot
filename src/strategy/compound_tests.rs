//! Unit tests for compound growth strategy

#[cfg(test)]
mod tests {
    use super::super::compound::*;
    use crate::config::{RiskConfig, StrategyConfig};
    
    use rust_decimal_macros::dec;

    fn make_test_config() -> (StrategyConfig, RiskConfig) {
        let strategy = StrategyConfig {
            min_edge: dec!(0.06),
            min_confidence: dec!(0.60),
            kelly_fraction: dec!(0.35),
            scan_interval_secs: 180,
            model_update_interval_secs: 900,
            compound_enabled: true,
            compound_sqrt_scaling: true,
        };
        
        let risk = RiskConfig {
            max_position_pct: dec!(0.08),
            max_exposure_pct: dec!(0.60),
            max_daily_loss_pct: dec!(0.12),
            min_balance_reserve: dec!(25),
            max_open_positions: 12,
        };
        
        (strategy, risk)
    }

    #[test]
    fn test_compound_strategy_creation() {
        let (strategy_config, risk_config) = make_test_config();
        let initial_balance = dec!(1000);
        
        let strategy = CompoundStrategy::new(strategy_config, risk_config, initial_balance);
        let stats = strategy.get_stats();
        
        assert_eq!(stats.total_trades, 0);
        assert_eq!(stats.win_streak, 0);
        assert_eq!(stats.lose_streak, 0);
        assert_eq!(stats.current_kelly_mult, dec!(1.0));
    }

    #[test]
    fn test_win_streak_increases_kelly() {
        let (strategy_config, risk_config) = make_test_config();
        let mut strategy = CompoundStrategy::new(strategy_config, risk_config, dec!(1000));
        
        // Record 5 wins
        for _ in 0..5 {
            strategy.record_result(dec!(100), dec!(0.10), dec!(0.70));
        }
        
        let stats = strategy.get_stats();
        assert_eq!(stats.win_streak, 5);
        assert_eq!(stats.lose_streak, 0);
    }

    #[test]
    fn test_lose_streak_resets_win_streak() {
        let (strategy_config, risk_config) = make_test_config();
        let mut strategy = CompoundStrategy::new(strategy_config, risk_config, dec!(1000));
        
        // Record 3 wins then 1 loss
        for _ in 0..3 {
            strategy.record_result(dec!(100), dec!(0.10), dec!(0.70));
        }
        strategy.record_result(dec!(-50), dec!(0.10), dec!(0.70));
        
        let stats = strategy.get_stats();
        assert_eq!(stats.win_streak, 0);
        assert_eq!(stats.lose_streak, 1);
    }

    #[test]
    fn test_balance_update_tracks_peak() {
        let (strategy_config, risk_config) = make_test_config();
        let mut strategy = CompoundStrategy::new(strategy_config, risk_config, dec!(1000));
        
        strategy.update_balance(dec!(1500));
        strategy.update_balance(dec!(1200));  // Drawdown
        
        let stats = strategy.get_stats();
        assert_eq!(stats.growth_from_initial, dec!(1.5));  // Peak was 1500
    }

    #[test]
    fn test_compound_stats_calculation() {
        let (strategy_config, risk_config) = make_test_config();
        let mut strategy = CompoundStrategy::new(strategy_config, risk_config, dec!(1000));
        
        // Record mixed results
        strategy.record_result(dec!(100), dec!(0.10), dec!(0.70));
        strategy.record_result(dec!(50), dec!(0.08), dec!(0.65));
        strategy.record_result(dec!(-30), dec!(0.12), dec!(0.60));
        
        let stats = strategy.get_stats();
        assert_eq!(stats.total_trades, 3);
        assert_eq!(stats.wins, 2);
        assert!((stats.win_rate - 0.666).abs() < 0.01);
        assert_eq!(stats.total_pnl, dec!(120));
    }
}
