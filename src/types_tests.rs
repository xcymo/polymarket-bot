//! Tests for core types

#[cfg(test)]
mod tests {
    use super::super::types::*;
    use chrono::Utc;
    use rust_decimal::Decimal;
    use rust_decimal_macros::dec;

    #[test]
    fn test_side_serialization() {
        assert_eq!(serde_json::to_string(&Side::Buy).unwrap(), "\"BUY\"");
        assert_eq!(serde_json::to_string(&Side::Sell).unwrap(), "\"SELL\"");
    }

    #[test]
    fn test_side_deserialization() {
        let buy: Side = serde_json::from_str("\"BUY\"").unwrap();
        let sell: Side = serde_json::from_str("\"SELL\"").unwrap();
        assert_eq!(buy, Side::Buy);
        assert_eq!(sell, Side::Sell);
    }

    #[test]
    fn test_order_type_values() {
        assert_ne!(OrderType::GTC, OrderType::FOK);
        assert_ne!(OrderType::FOK, OrderType::GTD);
        assert_ne!(OrderType::GTC, OrderType::GTD);
    }

    #[test]
    fn test_market_yes_price() {
        let market = create_test_market(dec!(0.65), dec!(0.35));
        assert_eq!(market.yes_price(), Some(dec!(0.65)));
    }

    #[test]
    fn test_market_no_price() {
        let market = create_test_market(dec!(0.65), dec!(0.35));
        assert_eq!(market.no_price(), Some(dec!(0.35)));
    }

    #[test]
    fn test_market_no_arbitrage() {
        let market = create_test_market(dec!(0.65), dec!(0.35));
        // 0.65 + 0.35 = 1.0, no arbitrage
        assert_eq!(market.arbitrage_opportunity(), None);
    }

    #[test]
    fn test_market_with_arbitrage() {
        let market = create_test_market(dec!(0.45), dec!(0.45));
        // 0.45 + 0.45 = 0.90, arbitrage of 0.10
        assert_eq!(market.arbitrage_opportunity(), Some(dec!(0.10)));
    }

    #[test]
    fn test_market_no_arbitrage_overpriced() {
        let market = create_test_market(dec!(0.55), dec!(0.50));
        // 0.55 + 0.50 = 1.05 > 1.0, no arbitrage opportunity
        assert_eq!(market.arbitrage_opportunity(), None);
    }

    #[test]
    fn test_signal_is_tradeable_true() {
        let signal = create_test_signal(dec!(0.08), dec!(0.75));
        assert!(signal.is_tradeable(dec!(0.05), dec!(0.60)));
    }

    #[test]
    fn test_signal_not_tradeable_low_edge() {
        let signal = create_test_signal(dec!(0.03), dec!(0.75));
        assert!(!signal.is_tradeable(dec!(0.05), dec!(0.60)));
    }

    #[test]
    fn test_signal_not_tradeable_low_confidence() {
        let signal = create_test_signal(dec!(0.08), dec!(0.50));
        assert!(!signal.is_tradeable(dec!(0.05), dec!(0.60)));
    }

    #[test]
    fn test_signal_tradeable_at_boundary() {
        let signal = create_test_signal(dec!(0.05), dec!(0.60));
        assert!(signal.is_tradeable(dec!(0.05), dec!(0.60)));
    }

    #[test]
    fn test_signal_negative_edge_tradeable() {
        // Negative edge (sell signal) should also be tradeable if abs(edge) >= min
        let signal = create_test_signal(dec!(-0.08), dec!(0.75));
        assert!(signal.is_tradeable(dec!(0.05), dec!(0.60)));
    }

    #[test]
    fn test_order_creation() {
        let order = Order {
            token_id: "token123".to_string(),
            side: Side::Buy,
            price: dec!(0.50),
            size: dec!(100),
            order_type: OrderType::GTC,
        };
        assert_eq!(order.token_id, "token123");
        assert_eq!(order.side, Side::Buy);
        assert_eq!(order.price, dec!(0.50));
        assert_eq!(order.size, dec!(100));
    }

    #[test]
    fn test_order_serialization() {
        let order = Order {
            token_id: "token123".to_string(),
            side: Side::Buy,
            price: dec!(0.50),
            size: dec!(100),
            order_type: OrderType::GTC,
        };
        let json = serde_json::to_string(&order).unwrap();
        assert!(json.contains("\"token_id\":\"token123\""));
        assert!(json.contains("\"side\":\"BUY\""));
    }

    #[test]
    fn test_position_creation() {
        let position = Position {
            token_id: "token123".to_string(),
            market_id: "market456".to_string(),
            side: Side::Buy,
            size: dec!(100),
            avg_entry_price: dec!(0.45),
            current_price: dec!(0.55),
            unrealized_pnl: dec!(10),
        };
        assert_eq!(position.unrealized_pnl, dec!(10));
    }

    #[test]
    fn test_trade_creation() {
        let trade = Trade {
            id: "trade1".to_string(),
            order_id: "order1".to_string(),
            token_id: "token123".to_string(),
            market_id: "market456".to_string(),
            side: Side::Buy,
            price: dec!(0.50),
            size: dec!(100),
            fee: dec!(0.50),
            timestamp: Utc::now(),
        };
        assert_eq!(trade.fee, dec!(0.50));
    }

    #[test]
    fn test_outcome_creation() {
        let outcome = Outcome {
            token_id: "token123".to_string(),
            outcome: "Yes".to_string(),
            price: dec!(0.65),
        };
        assert_eq!(outcome.outcome, "Yes");
        assert_eq!(outcome.price, dec!(0.65));
    }

    #[test]
    fn test_order_status_creation() {
        let status = OrderStatus {
            order_id: "order1".to_string(),
            status: "FILLED".to_string(),
            filled_size: dec!(100),
            remaining_size: dec!(0),
            avg_price: Some(dec!(0.50)),
        };
        assert_eq!(status.status, "FILLED");
        assert_eq!(status.filled_size, dec!(100));
    }

    #[test]
    fn test_market_empty_outcomes() {
        let market = Market {
            id: "test".to_string(),
            question: "Test?".to_string(),
            description: None,
            end_date: None,
            volume: dec!(0),
            liquidity: dec!(0),
            outcomes: vec![],
            active: true,
            closed: false,
        };
        assert_eq!(market.yes_price(), None);
        assert_eq!(market.no_price(), None);
        assert_eq!(market.arbitrage_opportunity(), None);
    }

    // Helper functions
    fn create_test_market(yes_price: Decimal, no_price: Decimal) -> Market {
        Market {
            id: "test-market".to_string(),
            question: "Test question?".to_string(),
            description: Some("Test description".to_string()),
            end_date: Some(Utc::now()),
            volume: dec!(10000),
            liquidity: dec!(5000),
            outcomes: vec![
                Outcome {
                    token_id: "yes-token".to_string(),
                    outcome: "Yes".to_string(),
                    price: yes_price,
                },
                Outcome {
                    token_id: "no-token".to_string(),
                    outcome: "No".to_string(),
                    price: no_price,
                },
            ],
            active: true,
            closed: false,
        }
    }

    fn create_test_signal(edge: Decimal, confidence: Decimal) -> Signal {
        Signal {
            market_id: "test-market".to_string(),
            token_id: "test-token".to_string(),
            side: Side::Buy,
            model_probability: dec!(0.70),
            market_probability: dec!(0.60),
            edge,
            confidence,
            suggested_size: dec!(100),
            timestamp: Utc::now(),
        }
    }
}
