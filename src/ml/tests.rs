//! Integration tests for ML module

use super::*;
use rust_decimal_macros::dec;
use chrono::Utc;

#[test]
fn test_full_ml_pipeline() {
    // 1. Feature extraction
    let mut extractor = FeatureExtractor::with_defaults();
    
    // Simulate market data
    for i in 0..20 {
        let price = dec!(0.5) + Decimal::from(i) * dec!(0.01);
        let volume = dec!(100) + Decimal::from(i % 5) * dec!(10);
        
        extractor.update(&features::DataPoint {
            timestamp: Utc::now(),
            price,
            volume,
            bid_price: Some(price - dec!(0.01)),
            ask_price: Some(price + dec!(0.01)),
            bid_size: Some(dec!(100)),
            ask_size: Some(dec!(100)),
        });
    }
    
    let current_point = features::DataPoint {
        timestamp: Utc::now(),
        price: dec!(0.69),
        volume: dec!(150),
        bid_price: Some(dec!(0.68)),
        ask_price: Some(dec!(0.70)),
        bid_size: Some(dec!(200)),
        ask_size: Some(dec!(150)),
    };
    
    let features = extractor.extract(None, &current_point);
    assert!(features.price_momentum_short > Decimal::ZERO);
    
    // 2. Model predictions with calibration
    let mut calibrator = ProbabilityCalibrator::with_platt_scaling();
    
    // Simulate training calibrator
    for _ in 0..30 {
        calibrator.add_sample(dec!(0.7), true);
        calibrator.add_sample(dec!(0.3), false);
    }
    
    let raw_prediction = dec!(0.75);
    let calibrated = calibrator.calibrate(raw_prediction);
    assert!(calibrated.calibrated_probability > dec!(0.5));
    
    // 3. Ensemble multiple models
    let mut ensemble = EnsemblePredictor::with_defaults();
    
    // Train model performance
    for _ in 0..30 {
        ensemble.record_outcome("llm_model", dec!(0.65), true);
        ensemble.record_outcome("technical_model", dec!(0.6), true);
    }
    
    let predictions = vec![
        ModelPrediction {
            model_id: "llm_model".to_string(),
            probability: dec!(0.7),
            confidence: dec!(0.85),
            uncertainty: Some(dec!(0.1)),
            timestamp: Utc::now(),
            metadata: None,
        },
        ModelPrediction {
            model_id: "technical_model".to_string(),
            probability: dec!(0.65),
            confidence: dec!(0.8),
            uncertainty: Some(dec!(0.12)),
            timestamp: Utc::now(),
            metadata: None,
        },
    ];
    
    let ensemble_result = ensemble.predict(&predictions).unwrap();
    assert!(ensemble_result.probability > dec!(0.6));
    assert!(ensemble_result.agreement > dec!(0.8));
    
    // 4. Multi-factor fusion
    let mut fusion = MultiFactorFusion::with_defaults();
    
    // Record factor performance
    for _ in 0..30 {
        fusion.record_outcome("technical", dec!(0.5), true);
        fusion.record_outcome("sentiment", dec!(0.3), true);
    }
    
    let factors = vec![
        Factor {
            id: "technical".to_string(),
            name: "Technical Analysis".to_string(),
            category: FactorCategory::Technical,
            value: features.trend_strength,
            signal: dec!(0.4),
            confidence: dec!(0.8),
            timestamp: Utc::now(),
        },
        Factor {
            id: "sentiment".to_string(),
            name: "Sentiment".to_string(),
            category: FactorCategory::Sentiment,
            value: dec!(0.3),
            signal: dec!(0.3),
            confidence: dec!(0.7),
            timestamp: Utc::now(),
        },
        Factor {
            id: "model".to_string(),
            name: "ML Model".to_string(),
            category: FactorCategory::Model,
            value: ensemble_result.probability,
            signal: (ensemble_result.probability - dec!(0.5)) * dec!(2.0),
            confidence: ensemble_result.confidence,
            timestamp: Utc::now(),
        },
    ];
    
    let fusion_result = fusion.fuse(&factors);
    
    // Final signal should incorporate all factors
    assert!(fusion_result.probability > dec!(0.5));
    assert!(fusion_result.diversity > Decimal::ZERO);
    assert_eq!(fusion_result.contributions.len(), 3);
}

#[test]
fn test_calibration_improves_brier_score() {
    let mut calibrator = ProbabilityCalibrator::with_platt_scaling();
    
    // Train with overconfident predictions
    for _ in 0..100 {
        // Model predicts 0.9, but only right 70% of time
        calibrator.add_sample(dec!(0.9), rand::random::<f32>() < 0.7);
    }
    
    calibrator.refit();
    
    // After calibration, 0.9 predictions should be pulled down
    let result = calibrator.calibrate(dec!(0.9));
    assert!(result.calibrated_probability < dec!(0.9));
}

#[test]
fn test_ensemble_adapts_to_model_performance() {
    let mut ensemble = EnsemblePredictor::with_defaults();
    
    // Model A is consistently good
    for _ in 0..50 {
        ensemble.record_outcome("model_a", dec!(0.7), true);
        ensemble.record_outcome("model_a", dec!(0.3), false);
    }
    
    // Model B is consistently bad
    for _ in 0..50 {
        ensemble.record_outcome("model_b", dec!(0.7), false);
        ensemble.record_outcome("model_b", dec!(0.3), true);
    }
    
    let weights = ensemble.get_model_weights();
    
    assert!(weights.get("model_a").unwrap() > weights.get("model_b").unwrap());
}

#[test]
fn test_factor_fusion_penalizes_poor_ic() {
    let mut fusion = MultiFactorFusion::with_defaults();
    
    // Good factor: high IC
    for _ in 0..50 {
        fusion.record_outcome("good_factor", dec!(0.5), true);
        fusion.record_outcome("good_factor", dec!(-0.5), false);
    }
    
    // Bad factor: negative IC
    for _ in 0..50 {
        fusion.record_outcome("bad_factor", dec!(0.5), false);
        fusion.record_outcome("bad_factor", dec!(-0.5), true);
    }
    
    let good_stats = fusion.factor_stats("good_factor").unwrap();
    let bad_stats = fusion.factor_stats("bad_factor").unwrap();
    
    assert!(good_stats.information_coefficient > Decimal::ZERO);
    assert!(bad_stats.information_coefficient < Decimal::ZERO);
}

#[test]
fn test_feature_extraction_handles_sparse_data() {
    let extractor = FeatureExtractor::with_defaults();
    
    // Single data point
    let point = features::DataPoint {
        timestamp: Utc::now(),
        price: dec!(0.5),
        volume: dec!(100),
        bid_price: None,
        ask_price: None,
        bid_size: None,
        ask_size: None,
    };
    
    let features = extractor.extract(None, &point);
    
    // Should handle gracefully
    assert!(features.data_completeness < Decimal::ONE);
    assert_eq!(features.bid_ask_spread, Decimal::ZERO);
}

// Helper for random in tests
mod rand {
    pub fn random<T>() -> f32 {
        use std::time::{SystemTime, UNIX_EPOCH};
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .subsec_nanos();
        (nanos % 100) as f32 / 100.0
    }
}

use rust_decimal::Decimal;
