//! Probability models for market analysis
//!
//! This module contains various models to estimate the "true" probability
//! of market outcomes, which can be compared to market prices to find edge.

mod llm;
mod sentiment;
#[cfg(test)]
mod tests;

pub use llm::{LlmModel, LlmProvider};
pub use sentiment::SentimentModel;

use crate::error::Result;
use crate::types::Market;
use async_trait::async_trait;
use rust_decimal::Decimal;

/// Probability prediction result
#[derive(Debug, Clone)]
pub struct Prediction {
    /// Estimated probability (0-1)
    pub probability: Decimal,
    /// Confidence in the prediction (0-1)
    pub confidence: Decimal,
    /// Reasoning/explanation
    pub reasoning: String,
}

/// Trait for probability models
#[async_trait]
pub trait ProbabilityModel: Send + Sync {
    /// Predict the probability of the "Yes" outcome
    async fn predict(&self, market: &Market) -> Result<Prediction>;
    
    /// Model name for logging
    fn name(&self) -> &str;
}

/// Ensemble model combining multiple models
pub struct EnsembleModel {
    models: Vec<(Box<dyn ProbabilityModel>, Decimal)>, // (model, weight)
}

impl EnsembleModel {
    pub fn new() -> Self {
        Self { models: Vec::new() }
    }

    pub fn add_model(&mut self, model: Box<dyn ProbabilityModel>, weight: Decimal) {
        self.models.push((model, weight));
    }

    pub async fn predict(&self, market: &Market) -> Result<Prediction> {
        if self.models.is_empty() {
            return Ok(Prediction {
                probability: Decimal::new(50, 2),
                confidence: Decimal::ZERO,
                reasoning: "No models configured".to_string(),
            });
        }

        let mut total_weight = Decimal::ZERO;
        let mut weighted_prob = Decimal::ZERO;
        let mut weighted_conf = Decimal::ZERO;
        let mut reasons = Vec::new();

        for (model, weight) in &self.models {
            match model.predict(market).await {
                Ok(pred) => {
                    weighted_prob += pred.probability * weight;
                    weighted_conf += pred.confidence * weight;
                    total_weight += weight;
                    reasons.push(format!("{}: {:.0}%", model.name(), pred.probability * Decimal::ONE_HUNDRED));
                }
                Err(e) => {
                    tracing::warn!("Model {} failed: {}", model.name(), e);
                }
            }
        }

        if total_weight == Decimal::ZERO {
            return Ok(Prediction {
                probability: Decimal::new(50, 2),
                confidence: Decimal::ZERO,
                reasoning: "All models failed".to_string(),
            });
        }

        Ok(Prediction {
            probability: weighted_prob / total_weight,
            confidence: weighted_conf / total_weight,
            reasoning: reasons.join("; "),
        })
    }
}

impl Default for EnsembleModel {
    fn default() -> Self {
        Self::new()
    }
}
