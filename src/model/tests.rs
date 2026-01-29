//! Tests for model module

#[cfg(test)]
mod tests {
    use super::super::llm::{LlmModel, LlmProvider};
    use crate::config::LlmConfig;
    use crate::types::{Market, Outcome};
    use chrono::Utc;
    use rust_decimal_macros::dec;

    #[test]
    fn test_llm_provider_deepseek() {
        let provider = LlmProvider::DeepSeek {
            api_key: "test-key".to_string(),
            model: "deepseek-chat".to_string(),
        };
        match provider {
            LlmProvider::DeepSeek { api_key, model } => {
                assert_eq!(api_key, "test-key");
                assert_eq!(model, "deepseek-chat");
            }
            _ => panic!("Expected DeepSeek provider"),
        }
    }

    #[test]
    fn test_llm_provider_anthropic() {
        let provider = LlmProvider::Anthropic {
            api_key: "test-key".to_string(),
            model: "claude-3".to_string(),
        };
        match provider {
            LlmProvider::Anthropic { api_key, model } => {
                assert_eq!(api_key, "test-key");
                assert_eq!(model, "claude-3");
            }
            _ => panic!("Expected Anthropic provider"),
        }
    }

    #[test]
    fn test_llm_provider_openai() {
        let provider = LlmProvider::OpenAI {
            api_key: "test-key".to_string(),
            model: "gpt-4".to_string(),
            base_url: "https://api.openai.com".to_string(),
        };
        match provider {
            LlmProvider::OpenAI { api_key, model, base_url } => {
                assert_eq!(api_key, "test-key");
                assert_eq!(model, "gpt-4");
                assert_eq!(base_url, "https://api.openai.com");
            }
            _ => panic!("Expected OpenAI provider"),
        }
    }

    #[test]
    fn test_llm_provider_compatible() {
        let provider = LlmProvider::Compatible {
            api_key: None,
            model: "llama3".to_string(),
            base_url: "http://localhost:11434".to_string(),
        };
        match provider {
            LlmProvider::Compatible { api_key, model, base_url } => {
                assert!(api_key.is_none());
                assert_eq!(model, "llama3");
                assert_eq!(base_url, "http://localhost:11434");
            }
            _ => panic!("Expected Compatible provider"),
        }
    }

    #[test]
    fn test_llm_model_new() {
        let model = LlmModel::new(LlmProvider::DeepSeek {
            api_key: "test".to_string(),
            model: "test".to_string(),
        });
        // Just verify it creates without panic
        assert!(true);
        let _ = model;
    }

    #[test]
    fn test_llm_model_deepseek_constructor() {
        let model = LlmModel::deepseek("sk-test".to_string());
        let _ = model;
    }

    #[test]
    fn test_llm_model_anthropic_constructor() {
        let model = LlmModel::anthropic("sk-test".to_string());
        let _ = model;
    }

    #[test]
    fn test_llm_model_openai_constructor() {
        let model = LlmModel::openai("sk-test".to_string());
        let _ = model;
    }

    #[test]
    fn test_llm_model_from_config_deepseek() {
        let config = LlmConfig {
            provider: "deepseek".to_string(),
            api_key: "sk-test".to_string(),
            model: None,
            base_url: None,
        };
        let model = LlmModel::from_config(&config).unwrap();
        let _ = model;
    }

    #[test]
    fn test_llm_model_from_config_anthropic() {
        let config = LlmConfig {
            provider: "anthropic".to_string(),
            api_key: "sk-test".to_string(),
            model: Some("claude-3".to_string()),
            base_url: None,
        };
        let model = LlmModel::from_config(&config).unwrap();
        let _ = model;
    }

    #[test]
    fn test_llm_model_from_config_claude_alias() {
        let config = LlmConfig {
            provider: "claude".to_string(),
            api_key: "sk-test".to_string(),
            model: None,
            base_url: None,
        };
        let model = LlmModel::from_config(&config).unwrap();
        let _ = model;
    }

    #[test]
    fn test_llm_model_from_config_openai() {
        let config = LlmConfig {
            provider: "openai".to_string(),
            api_key: "sk-test".to_string(),
            model: Some("gpt-4".to_string()),
            base_url: Some("https://api.openai.com".to_string()),
        };
        let model = LlmModel::from_config(&config).unwrap();
        let _ = model;
    }

    #[test]
    fn test_llm_model_from_config_gpt_alias() {
        let config = LlmConfig {
            provider: "gpt".to_string(),
            api_key: "sk-test".to_string(),
            model: None,
            base_url: None,
        };
        let model = LlmModel::from_config(&config).unwrap();
        let _ = model;
    }

    #[test]
    fn test_llm_model_from_config_ollama() {
        let config = LlmConfig {
            provider: "ollama".to_string(),
            api_key: "".to_string(),
            model: None,
            base_url: None,
        };
        let model = LlmModel::from_config(&config).unwrap();
        let _ = model;
    }

    #[test]
    fn test_llm_model_from_config_compatible() {
        let config = LlmConfig {
            provider: "compatible".to_string(),
            api_key: "".to_string(),
            model: Some("custom-model".to_string()),
            base_url: Some("http://localhost:8000".to_string()),
        };
        let model = LlmModel::from_config(&config).unwrap();
        let _ = model;
    }

    #[test]
    fn test_llm_model_from_config_custom_alias() {
        let config = LlmConfig {
            provider: "custom".to_string(),
            api_key: "test-key".to_string(),
            model: Some("custom-model".to_string()),
            base_url: Some("http://localhost:8000".to_string()),
        };
        let model = LlmModel::from_config(&config).unwrap();
        let _ = model;
    }

    #[test]
    fn test_llm_model_from_config_unknown_provider() {
        let config = LlmConfig {
            provider: "unknown".to_string(),
            api_key: "test".to_string(),
            model: None,
            base_url: None,
        };
        let result = LlmModel::from_config(&config);
        assert!(result.is_err());
    }

    #[test]
    fn test_llm_model_from_config_compatible_missing_model() {
        let config = LlmConfig {
            provider: "compatible".to_string(),
            api_key: "".to_string(),
            model: None,
            base_url: Some("http://localhost:8000".to_string()),
        };
        let result = LlmModel::from_config(&config);
        assert!(result.is_err());
    }

    #[test]
    fn test_llm_model_from_config_compatible_missing_base_url() {
        let config = LlmConfig {
            provider: "compatible".to_string(),
            api_key: "".to_string(),
            model: Some("model".to_string()),
            base_url: None,
        };
        let result = LlmModel::from_config(&config);
        assert!(result.is_err());
    }

    #[test]
    fn test_llm_model_from_config_case_insensitive() {
        let config = LlmConfig {
            provider: "DEEPSEEK".to_string(),
            api_key: "test".to_string(),
            model: None,
            base_url: None,
        };
        let result = LlmModel::from_config(&config);
        assert!(result.is_ok());
    }

    // Test prompt building
    fn create_test_market() -> Market {
        Market {
            id: "test-id".to_string(),
            question: "Will it rain tomorrow?".to_string(),
            description: Some("Weather prediction market".to_string()),
            end_date: Some(Utc::now()),
            volume: dec!(1000),
            liquidity: dec!(500),
            outcomes: vec![
                Outcome {
                    token_id: "yes-token".to_string(),
                    outcome: "Yes".to_string(),
                    price: dec!(0.65),
                },
                Outcome {
                    token_id: "no-token".to_string(),
                    outcome: "No".to_string(),
                    price: dec!(0.35),
                },
            ],
            active: true,
            closed: false,
        }
    }

    #[test]
    fn test_market_for_prompt() {
        let market = create_test_market();
        assert_eq!(market.yes_price(), Some(dec!(0.65)));
        assert_eq!(market.no_price(), Some(dec!(0.35)));
    }
}
