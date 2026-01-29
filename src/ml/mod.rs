//! Machine Learning prediction module
//!
//! Provides ML-based probability prediction with:
//! - Feature engineering from market data
//! - Probability calibration (Platt scaling, isotonic)
//! - Ensemble prediction combining multiple models
//! - Multi-factor fusion with dynamic weighting
//! - Unified predictor interface for live trading

pub mod features;
pub mod calibration;
pub mod ensemble;
pub mod factors;
pub mod predictor;

#[cfg(test)]
mod tests;

pub use features::{FeatureExtractor, MarketFeatures, FeatureConfig};
pub use calibration::{ProbabilityCalibrator, CalibrationMethod, CalibrationResult};
pub use ensemble::{EnsemblePredictor, ModelPrediction, EnsembleConfig, EnsembleMethod};
pub use factors::{MultiFactorFusion, Factor, FactorWeight, FusionConfig, FusionResult, FactorCategory};
pub use predictor::{MLPredictor, MLPredictorConfig, MLPredictionResult, MarketDataInput, KlineData, FeatureSummary};
