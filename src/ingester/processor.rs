//! Signal processing and aggregation
//!
//! Uses LLM to extract structured signals from raw messages,
//! then aggregates multi-source signals for validation.

use super::{ActionType, ParsedSignal, RawSignal, SignalDirection};
use crate::config::LlmConfig;
use crate::error::{BotError, Result};
use chrono::{Duration, Utc};
use reqwest::Client;
use std::collections::HashMap;
use tokio::sync::mpsc;

/// Signal processor using LLM for extraction
pub struct SignalProcessor {
    http: Client,
    llm_config: LlmConfig,
    /// Aggregation window in seconds
    aggregation_window: i64,
    /// Minimum confidence to emit signal
    min_confidence: f64,
    /// Minimum aggregate score to emit
    min_agg_score: f64,
}

impl SignalProcessor {
    pub fn new(llm_config: LlmConfig) -> Self {
        Self {
            http: Client::new(),
            llm_config,
            aggregation_window: 300, // 5 minutes
            min_confidence: 0.5,
            min_agg_score: 0.6,
        }
    }

    pub fn with_thresholds(mut self, min_confidence: f64, min_agg_score: f64) -> Self {
        self.min_confidence = min_confidence;
        self.min_agg_score = min_agg_score;
        self
    }

    pub fn with_window(mut self, seconds: i64) -> Self {
        self.aggregation_window = seconds;
        self
    }

    /// Run the processing pipeline
    pub async fn run(
        &self,
        mut raw_rx: mpsc::Receiver<RawSignal>,
        parsed_tx: mpsc::Sender<ParsedSignal>,
    ) -> Result<()> {
        // Buffer for aggregation
        let mut signal_buffer: HashMap<String, Vec<ExtractedSignal>> = HashMap::new();
        let mut cleanup_interval = tokio::time::interval(std::time::Duration::from_secs(60));

        loop {
            tokio::select! {
                Some(raw) = raw_rx.recv() => {
                    // Extract signal using LLM
                    match self.extract_signal(&raw).await {
                        Ok(Some(extracted)) => {
                            tracing::debug!(
                                "Extracted signal: {} {} (conf: {:.2})",
                                extracted.token,
                                match extracted.direction {
                                    SignalDirection::Bullish => "📈",
                                    SignalDirection::Bearish => "📉",
                                    SignalDirection::Neutral => "➡️",
                                },
                                extracted.confidence
                            );

                            // Add to buffer
                            let key = extracted.token.clone();
                            signal_buffer
                                .entry(key.clone())
                                .or_default()
                                .push(extracted);

                            // Try to aggregate
                            if let Some(aggregated) = self.try_aggregate(&key, &mut signal_buffer) {
                                if aggregated.agg_score >= self.min_agg_score {
                                    tracing::info!(
                                        "🎯 Aggregated signal: {} {} score={:.2}",
                                        aggregated.token,
                                        match aggregated.direction {
                                            SignalDirection::Bullish => "BULLISH",
                                            SignalDirection::Bearish => "BEARISH",
                                            SignalDirection::Neutral => "NEUTRAL",
                                        },
                                        aggregated.agg_score
                                    );

                                    if parsed_tx.send(aggregated).await.is_err() {
                                        tracing::warn!("Parsed signal channel closed");
                                        return Ok(());
                                    }
                                }
                            }
                        }
                        Ok(None) => {
                            tracing::debug!("No signal extracted from: {}", raw.source_id);
                        }
                        Err(e) => {
                            tracing::warn!("Failed to extract signal: {}", e);
                        }
                    }
                }
                _ = cleanup_interval.tick() => {
                    // Remove old signals from buffer
                    let cutoff = Utc::now() - Duration::seconds(self.aggregation_window * 2);
                    for signals in signal_buffer.values_mut() {
                        signals.retain(|s| s.timestamp > cutoff);
                    }
                    signal_buffer.retain(|_, v| !v.is_empty());
                }
            }
        }
    }

    /// Extract structured signal from raw message using LLM
    async fn extract_signal(&self, raw: &RawSignal) -> Result<Option<ExtractedSignal>> {
        let prompt = format!(
            r#"Analyze this crypto/trading message and extract signal information.

Source: {}
Author: {} (trust score: {:.2})
Content: {}

If this contains a trading signal or market insight, respond with JSON:
{{
  "token": "BTC/ETH/SOL/etc or null if not about specific token",
  "direction": "bullish/bearish/neutral",
  "confidence": 0.0-1.0,
  "timeframe": "5m/15m/1h/4h/1d or null",
  "action": "entry/exit/warning/info",
  "reasoning": "brief explanation"
}}

If this is NOT a trading-related message, respond with: {{"token": null}}

Only valid JSON, no other text."#,
            raw.source,
            raw.author,
            raw.author_trust,
            raw.content
        );

        let response = self.call_llm(&prompt).await?;
        let parsed = self.parse_llm_response(&response, raw)?;
        Ok(parsed)
    }

    async fn call_llm(&self, prompt: &str) -> Result<String> {
        let (base_url, model) = match self.llm_config.provider.to_lowercase().as_str() {
            "deepseek" => (
                "https://api.deepseek.com".to_string(),
                self.llm_config.model.clone().unwrap_or_else(|| "deepseek-chat".to_string()),
            ),
            "openai" | "gpt" => (
                self.llm_config.base_url.clone().unwrap_or_else(|| "https://api.openai.com".to_string()),
                self.llm_config.model.clone().unwrap_or_else(|| "gpt-4o-mini".to_string()),
            ),
            "ollama" => (
                self.llm_config.base_url.clone().unwrap_or_else(|| "http://localhost:11434".to_string()),
                self.llm_config.model.clone().unwrap_or_else(|| "qwen2.5:14b".to_string()),
            ),
            _ => (
                self.llm_config.base_url.clone().unwrap_or_else(|| "https://api.deepseek.com".to_string()),
                self.llm_config.model.clone().unwrap_or_else(|| "deepseek-chat".to_string()),
            ),
        };

        let request = serde_json::json!({
            "model": model,
            "messages": [{"role": "user", "content": prompt}],
            "response_format": {"type": "json_object"}
        });

        let mut req = self.http
            .post(format!("{}/v1/chat/completions", base_url))
            .header("content-type", "application/json");

        if !self.llm_config.api_key.is_empty() {
            req = req.header("Authorization", format!("Bearer {}", self.llm_config.api_key));
        }

        let resp: serde_json::Value = req.json(&request).send().await?.json().await?;

        resp["choices"][0]["message"]["content"]
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| BotError::Api("Empty LLM response".into()))
    }

    fn parse_llm_response(&self, response: &str, raw: &RawSignal) -> Result<Option<ExtractedSignal>> {
        // Extract JSON from response
        let json_str = if response.contains('{') {
            let start = response.find('{').unwrap();
            let end = response.rfind('}').unwrap_or(response.len() - 1) + 1;
            &response[start..end]
        } else {
            response
        };

        let parsed: serde_json::Value = serde_json::from_str(json_str)
            .map_err(|e| BotError::Api(format!("Failed to parse LLM response: {}", e)))?;

        // Check if this is a trading signal
        let token = match parsed["token"].as_str() {
            Some(t) if !t.is_empty() && t != "null" => t.to_uppercase(),
            _ => return Ok(None),
        };

        let direction = match parsed["direction"].as_str().unwrap_or("neutral") {
            "bullish" => SignalDirection::Bullish,
            "bearish" => SignalDirection::Bearish,
            _ => SignalDirection::Neutral,
        };

        let confidence = parsed["confidence"].as_f64().unwrap_or(0.5);
        if confidence < self.min_confidence {
            return Ok(None);
        }

        let action = match parsed["action"].as_str().unwrap_or("info") {
            "entry" => ActionType::Entry,
            "exit" => ActionType::Exit,
            "warning" => ActionType::Warning,
            _ => ActionType::Info,
        };

        Ok(Some(ExtractedSignal {
            token,
            direction,
            timeframe: parsed["timeframe"].as_str().unwrap_or("1h").to_string(),
            confidence,
            action,
            reasoning: parsed["reasoning"].as_str().unwrap_or("").to_string(),
            raw: raw.clone(),
            timestamp: Utc::now(),
        }))
    }

    /// Try to aggregate signals for a token
    fn try_aggregate(
        &self,
        token: &str,
        buffer: &mut HashMap<String, Vec<ExtractedSignal>>,
    ) -> Option<ParsedSignal> {
        // Take ownership of signals to avoid borrow issues
        let signals = buffer.remove(token)?;
        
        let cutoff = Utc::now() - Duration::seconds(self.aggregation_window);
        let recent: Vec<_> = signals.into_iter().filter(|s| s.timestamp > cutoff).collect();

        if recent.is_empty() {
            return None;
        }

        // Need at least 2 signals for aggregation, or 1 high-confidence signal
        if recent.len() < 2 {
            let first = &recent[0];
            if first.confidence < 0.8 || first.raw.author_trust < 0.7 {
                // Put back if not ready to aggregate
                buffer.insert(token.to_string(), recent);
                return None;
            }
        }

        // Count directions
        let mut bullish_score = 0.0;
        let mut bearish_score = 0.0;
        let mut sources = Vec::new();
        let mut best_timeframe = String::from("1h");
        let mut best_confidence = 0.0;
        let mut best_reasoning = String::new();
        let mut best_action = ActionType::Info;

        for s in &recent {
            let weight = s.confidence * s.raw.author_trust;
            match s.direction {
                SignalDirection::Bullish => bullish_score += weight,
                SignalDirection::Bearish => bearish_score += weight,
                SignalDirection::Neutral => {}
            }
            sources.push(s.raw.clone());
            
            // Track best signal
            if s.confidence > best_confidence {
                best_confidence = s.confidence;
                best_timeframe = s.timeframe.clone();
                best_reasoning = s.reasoning.clone();
                best_action = s.action;
            }
        }

        let total_score = bullish_score + bearish_score;
        if total_score < 0.3 {
            return None;
        }

        let (direction, agg_score) = if bullish_score > bearish_score {
            (SignalDirection::Bullish, bullish_score / total_score)
        } else if bearish_score > bullish_score {
            (SignalDirection::Bearish, bearish_score / total_score)
        } else {
            (SignalDirection::Neutral, 0.5)
        };

        // Multi-source bonus
        let unique_authors: std::collections::HashSet<_> = 
            sources.iter().map(|s| &s.author).collect();
        let multi_source_bonus = (unique_authors.len() as f64 - 1.0) * 0.1;
        let final_score = (agg_score + multi_source_bonus).min(1.0);

        Some(ParsedSignal {
            token: token.to_string(),
            direction,
            timeframe: best_timeframe,
            confidence: best_confidence,
            reasoning: best_reasoning,
            action_type: best_action,
            sources,
            agg_score: final_score,
            timestamp: Utc::now(),
        })
    }
}

/// Intermediate extracted signal before aggregation
#[derive(Debug, Clone)]
struct ExtractedSignal {
    token: String,
    direction: SignalDirection,
    timeframe: String,
    confidence: f64,
    action: ActionType,
    reasoning: String,
    raw: RawSignal,
    timestamp: chrono::DateTime<Utc>,
}
