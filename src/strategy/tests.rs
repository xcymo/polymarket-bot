//! Unit tests for strategy module

#[cfg(test)]
mod tests {
    use super::super::*;
    use crate::config::{RiskConfig, StrategyConfig};
    use crate::model::Prediction;
    use crate::types::{Market, Outcome, Side};
    use rust_decimal::Decimal;
    use rust_decimal_macros::dec;

    fn make_test_config() -> (StrategyConfig, RiskConfig) {
        let strategy = StrategyConfig {
            min_edge: dec!(0.05),
            min_confidence: dec!(0.6),
            kelly_fraction: dec!(0.25),
            scan_interval_secs: 60,
            model_update_interval_secs: 3600,
            compound_enabled: false,
            compound_sqrt_scaling: false,
        };
        
        let risk = RiskConfig {
            max_position_pct: dec!(0.05),
            max_exposure_pct: dec!(0.5),
            max_daily_loss_pct: dec!(0.1),
            min_balance_reserve: dec!(100),
            max_open_positions: 10,
        };
        
        (strategy, risk)
    }

    fn make_test_market(yes_price: Decimal) -> Market {
        Market {
            id: "test-market-1".to_string(),
            question: "Will test pass?".to_string(),
            description: Some("Test market".to_string()),
            end_date: None,
            volume: dec!(100000),
            liquidity: dec!(50000),
            active: true,
            closed: false,
            outcomes: vec![
                Outcome {
                    outcome: "Yes".to_string(),
                    token_id: "token-yes".to_string(),
                    price: yes_price,
                },
                Outcome {
                    outcome: "No".to_string(),
                    token_id: "token-no".to_string(),
                    price: Decimal::ONE - yes_price,
                },
            ],
        }
    }

    #[test]
    fn test_signal_generation_with_edge() {
        let (strategy_config, risk_config) = make_test_config();
        let signal_gen = SignalGenerator::new(strategy_config, risk_config);
        
        // Market at 40%, model thinks 55% -> positive edge
        let market = make_test_market(dec!(0.40));
        let prediction = Prediction {
            probability: dec!(0.55),
            confidence: dec!(0.70),
            reasoning: "Test".to_string(),
        };
        
        let signal = signal_gen.generate(&market, &prediction);
        assert!(signal.is_some(), "Should generate signal with 15% edge");
        
        let s = signal.unwrap();
        assert_eq!(s.side, Side::Buy);
        assert!(s.edge > dec!(0.10), "Edge should be > 10%");
    }

    #[test]
    fn test_no_signal_without_edge() {
        let (strategy_config, risk_config) = make_test_config();
        let signal_gen = SignalGenerator::new(strategy_config, risk_config);
        
        // Market at 50%, model thinks 52% -> small edge, below threshold
        let market = make_test_market(dec!(0.50));
        let prediction = Prediction {
            probability: dec!(0.52),
            confidence: dec!(0.70),
            reasoning: "Test".to_string(),
        };
        
        let signal = signal_gen.generate(&market, &prediction);
        assert!(signal.is_none(), "Should not generate signal with only 2% edge");
    }

    #[test]
    fn test_no_signal_low_confidence() {
        let (strategy_config, risk_config) = make_test_config();
        let signal_gen = SignalGenerator::new(strategy_config, risk_config);
        
        // Good edge but low confidence
        let market = make_test_market(dec!(0.40));
        let prediction = Prediction {
            probability: dec!(0.60),
            confidence: dec!(0.50), // Below threshold
            reasoning: "Test".to_string(),
        };
        
        let signal = signal_gen.generate(&market, &prediction);
        assert!(signal.is_none(), "Should not generate signal with low confidence");
    }

    #[test]
    fn test_sell_signal_when_overpriced() {
        let (strategy_config, risk_config) = make_test_config();
        let signal_gen = SignalGenerator::new(strategy_config, risk_config);
        
        // Market at 70%, model thinks 55% -> negative edge, should sell
        let market = make_test_market(dec!(0.70));
        let prediction = Prediction {
            probability: dec!(0.55),
            confidence: dec!(0.75),
            reasoning: "Test".to_string(),
        };
        
        let signal = signal_gen.generate(&market, &prediction);
        assert!(signal.is_some(), "Should generate sell signal");
        
        let s = signal.unwrap();
        assert_eq!(s.side, Side::Sell);
    }

    #[test]
    fn test_position_sizing_kelly() {
        let (strategy_config, risk_config) = make_test_config();
        let signal_gen = SignalGenerator::new(strategy_config, risk_config);
        
        // Large edge should result in larger position
        let market = make_test_market(dec!(0.30));
        let prediction = Prediction {
            probability: dec!(0.60),
            confidence: dec!(0.80),
            reasoning: "Test".to_string(),
        };
        
        let signal = signal_gen.generate(&market, &prediction).unwrap();
        
        // Size should be reasonable (0 < size <= max_position_pct)
        assert!(signal.suggested_size > Decimal::ZERO, "Size should be positive");
        assert!(signal.suggested_size <= dec!(0.05), "Size should not exceed max");
    }

    #[test]
    fn test_market_without_yes_outcome() {
        let (strategy_config, risk_config) = make_test_config();
        let signal_gen = SignalGenerator::new(strategy_config, risk_config);

        let market = Market {
            id: "test".to_string(),
            question: "Test?".to_string(),
            description: None,
            end_date: None,
            volume: dec!(0),
            liquidity: dec!(0),
            outcomes: vec![], // No outcomes
            active: true,
            closed: false,
        };
        let prediction = Prediction {
            probability: dec!(0.70),
            confidence: dec!(0.80),
            reasoning: "Test".to_string(),
        };

        let signal = signal_gen.generate(&market, &prediction);
        assert!(signal.is_none());
    }

    #[test]
    fn test_edge_exactly_at_threshold() {
        let (mut strategy_config, risk_config) = make_test_config();
        strategy_config.min_edge = dec!(0.05);
        let signal_gen = SignalGenerator::new(strategy_config, risk_config);

        let market = make_test_market(dec!(0.50));
        let prediction = Prediction {
            probability: dec!(0.55), // Exactly 5% edge
            confidence: dec!(0.80),
            reasoning: "Test".to_string(),
        };

        let signal = signal_gen.generate(&market, &prediction);
        assert!(signal.is_some());
    }

    #[test]
    fn test_large_positive_edge() {
        let (strategy_config, risk_config) = make_test_config();
        let signal_gen = SignalGenerator::new(strategy_config, risk_config);

        let market = make_test_market(dec!(0.20));
        let prediction = Prediction {
            probability: dec!(0.80), // 60% edge!
            confidence: dec!(0.90),
            reasoning: "Test".to_string(),
        };

        let signal = signal_gen.generate(&market, &prediction);
        assert!(signal.is_some());
        
        let s = signal.unwrap();
        assert_eq!(s.edge, dec!(0.60));
        assert_eq!(s.side, Side::Buy);
    }

    #[test]
    fn test_large_negative_edge() {
        let (strategy_config, risk_config) = make_test_config();
        let signal_gen = SignalGenerator::new(strategy_config, risk_config);

        let market = make_test_market(dec!(0.80));
        let prediction = Prediction {
            probability: dec!(0.20), // -60% edge
            confidence: dec!(0.90),
            reasoning: "Test".to_string(),
        };

        let signal = signal_gen.generate(&market, &prediction);
        assert!(signal.is_some());
        
        let s = signal.unwrap();
        assert_eq!(s.edge, dec!(-0.60));
        assert_eq!(s.side, Side::Sell);
    }

    #[test]
    fn test_signal_market_id() {
        let (strategy_config, risk_config) = make_test_config();
        let signal_gen = SignalGenerator::new(strategy_config, risk_config);

        let mut market = make_test_market(dec!(0.40));
        market.id = "unique-market-123".to_string();
        let prediction = Prediction {
            probability: dec!(0.55),
            confidence: dec!(0.75),
            reasoning: "Test".to_string(),
        };

        let signal = signal_gen.generate(&market, &prediction).unwrap();
        assert_eq!(signal.market_id, "unique-market-123");
    }

    #[test]
    fn test_signal_token_id() {
        let (strategy_config, risk_config) = make_test_config();
        let signal_gen = SignalGenerator::new(strategy_config, risk_config);

        let market = make_test_market(dec!(0.40));
        let prediction = Prediction {
            probability: dec!(0.55),
            confidence: dec!(0.75),
            reasoning: "Test".to_string(),
        };

        let signal = signal_gen.generate(&market, &prediction).unwrap();
        assert_eq!(signal.token_id, "token-yes");
    }

    #[test]
    fn test_prediction_fields_in_signal() {
        let (strategy_config, risk_config) = make_test_config();
        let signal_gen = SignalGenerator::new(strategy_config, risk_config);

        let market = make_test_market(dec!(0.45));
        let prediction = Prediction {
            probability: dec!(0.60),
            confidence: dec!(0.85),
            reasoning: "Test".to_string(),
        };

        let signal = signal_gen.generate(&market, &prediction).unwrap();
        assert_eq!(signal.model_probability, dec!(0.60));
        assert_eq!(signal.market_probability, dec!(0.45));
        assert_eq!(signal.confidence, dec!(0.85));
    }

    #[test]
    fn test_zero_edge() {
        let (strategy_config, risk_config) = make_test_config();
        let signal_gen = SignalGenerator::new(strategy_config, risk_config);

        let market = make_test_market(dec!(0.50));
        let prediction = Prediction {
            probability: dec!(0.50), // Zero edge
            confidence: dec!(0.80),
            reasoning: "Test".to_string(),
        };

        let signal = signal_gen.generate(&market, &prediction);
        assert!(signal.is_none()); // No edge = no signal
    }

    #[test]
    fn test_very_low_market_price() {
        let (strategy_config, risk_config) = make_test_config();
        let signal_gen = SignalGenerator::new(strategy_config, risk_config);

        let market = make_test_market(dec!(0.05));
        let prediction = Prediction {
            probability: dec!(0.20),
            confidence: dec!(0.80),
            reasoning: "Test".to_string(),
        };

        let signal = signal_gen.generate(&market, &prediction);
        assert!(signal.is_some());
        assert_eq!(signal.unwrap().side, Side::Buy);
    }

    #[test]
    fn test_very_high_market_price() {
        let (strategy_config, risk_config) = make_test_config();
        let signal_gen = SignalGenerator::new(strategy_config, risk_config);

        let market = make_test_market(dec!(0.95));
        let prediction = Prediction {
            probability: dec!(0.80),
            confidence: dec!(0.80),
            reasoning: "Test".to_string(),
        };

        let signal = signal_gen.generate(&market, &prediction);
        assert!(signal.is_some());
        assert_eq!(signal.unwrap().side, Side::Sell);
    }
}
