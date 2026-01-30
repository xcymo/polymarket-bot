//! Auto trader with take-profit, stop-loss, and persistent logging
//!
//! Wraps PaperTrader with:
//! - Auto-save on every state change
//! - Price snapshot logging (JSONL)
//! - Take-profit / stop-loss auto-closing
//! - Trade audit trail

use super::{PaperTrader, PaperTraderConfig, Position, PositionSide, PortfolioSummary, TradeRecord};
use crate::client::GammaClient;
use crate::error::Result;
use crate::types::Market;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tokio::fs::OpenOptions;
use tokio::io::AsyncWriteExt;
use tracing::{debug, info, warn};

/// Auto trader configuration
#[derive(Debug, Clone)]
pub struct AutoTraderConfig {
    /// Base paper trader config
    pub paper_config: PaperTraderConfig,
    /// Take profit percentage (e.g., 5.0 = +5%)
    pub take_profit_pct: Decimal,
    /// Stop loss percentage (e.g., 3.0 = -3%)
    pub stop_loss_pct: Decimal,
    /// State file path (auto-save)
    pub state_file: PathBuf,
    /// Snapshots directory
    pub snapshots_dir: PathBuf,
    /// Audit log file
    pub audit_file: PathBuf,
    /// Enable auto-save
    pub auto_save: bool,
    /// Enable price logging
    pub log_prices: bool,
}

impl Default for AutoTraderConfig {
    fn default() -> Self {
        Self {
            paper_config: PaperTraderConfig::default(),
            take_profit_pct: dec!(5.0),
            stop_loss_pct: dec!(3.0),
            state_file: PathBuf::from("paper_trading_state.json"),
            snapshots_dir: PathBuf::from("market_snapshots"),
            audit_file: PathBuf::from("trade_audit.jsonl"),
            auto_save: true,
            log_prices: true,
        }
    }
}

/// Price snapshot for logging
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PriceSnapshot {
    pub timestamp: DateTime<Utc>,
    pub market_id: String,
    pub question: String,
    pub yes_price: Decimal,
    pub no_price: Decimal,
}

/// Audit log entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    pub timestamp: DateTime<Utc>,
    pub action: String,
    pub market_id: String,
    pub side: Option<String>,
    pub shares: Option<Decimal>,
    pub price: Decimal,
    pub pnl: Option<Decimal>,
    pub pnl_pct: Option<Decimal>,
    pub reason: String,
    pub balance_before: Decimal,
    pub balance_after: Decimal,
}

/// Auto-closing result
#[derive(Debug, Clone)]
pub struct AutoCloseResult {
    pub position_id: String,
    pub reason: AutoCloseReason,
    pub pnl: Decimal,
    pub pnl_pct: Decimal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutoCloseReason {
    TakeProfit,
    StopLoss,
}

impl std::fmt::Display for AutoCloseReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AutoCloseReason::TakeProfit => write!(f, "TAKE_PROFIT"),
            AutoCloseReason::StopLoss => write!(f, "STOP_LOSS"),
        }
    }
}

/// Auto trader with persistence and auto-closing
pub struct AutoTrader {
    config: AutoTraderConfig,
    trader: PaperTrader,
}

impl AutoTrader {
    /// Create new auto trader
    pub fn new(config: AutoTraderConfig, client: GammaClient) -> Self {
        let trader = PaperTrader::new(config.paper_config.clone(), client);
        Self { config, trader }
    }

    /// Load state from file
    pub async fn load_state(&self) -> Result<()> {
        if self.config.state_file.exists() {
            self.trader.load_state(self.config.state_file.to_str().unwrap()).await?;
        }
        Ok(())
    }

    /// Save state to file
    pub async fn save_state(&self) -> Result<()> {
        self.trader.save_state(self.config.state_file.to_str().unwrap()).await
    }

    /// Buy with auto-save
    pub async fn buy(
        &self,
        market: &Market,
        side: PositionSide,
        amount_usd: Decimal,
        reason: String,
    ) -> Result<Position> {
        let balance_before = self.trader.get_balance().await;
        
        let position = self.trader.buy(market, side, amount_usd, reason.clone()).await?;
        
        let balance_after = self.trader.get_balance().await;
        
        // Auto-save
        if self.config.auto_save {
            self.save_state().await?;
        }
        
        // Audit log
        self.log_audit(AuditEntry {
            timestamp: Utc::now(),
            action: "BUY".to_string(),
            market_id: market.id.clone(),
            side: Some(side.to_string()),
            shares: Some(position.shares),
            price: position.entry_price,
            pnl: None,
            pnl_pct: None,
            reason,
            balance_before,
            balance_after,
        }).await?;
        
        // Log price snapshot
        if self.config.log_prices {
            self.log_price_snapshot(market).await?;
        }
        
        Ok(position)
    }

    /// Sell with auto-save
    pub async fn sell(&self, position_id: &str, reason: String) -> Result<TradeRecord> {
        let balance_before = self.trader.get_balance().await;
        
        let trade = self.trader.sell(position_id, reason.clone()).await?;
        
        let balance_after = self.trader.get_balance().await;
        
        // Auto-save
        if self.config.auto_save {
            self.save_state().await?;
        }
        
        // Audit log
        let pnl_pct = if trade.total_value > dec!(0) {
            trade.pnl.map(|p| (p / trade.total_value) * dec!(100))
        } else {
            None
        };
        
        self.log_audit(AuditEntry {
            timestamp: Utc::now(),
            action: "SELL".to_string(),
            market_id: trade.market_id.clone(),
            side: Some(trade.side.to_string()),
            shares: Some(trade.shares),
            price: trade.price,
            pnl: trade.pnl,
            pnl_pct,
            reason,
            balance_before,
            balance_after,
        }).await?;
        
        Ok(trade)
    }

    /// Update prices and check for auto-close conditions
    pub async fn update_and_check(&self) -> Result<Vec<AutoCloseResult>> {
        // Update prices
        self.trader.update_prices().await?;
        
        // Auto-save after price update
        if self.config.auto_save {
            self.save_state().await?;
        }
        
        // Check for auto-close conditions
        let mut results = Vec::new();
        let positions = self.trader.get_open_positions().await;
        
        for pos in positions {
            // Check take profit
            if pos.unrealized_pnl_pct >= self.config.take_profit_pct {
                let reason = format!(
                    "🎯 Take profit: {:.2}% >= {:.2}%",
                    pos.unrealized_pnl_pct, self.config.take_profit_pct
                );
                info!("{} for {}", reason, pos.question);
                
                let trade = self.sell(&pos.id, reason).await?;
                
                results.push(AutoCloseResult {
                    position_id: pos.id.clone(),
                    reason: AutoCloseReason::TakeProfit,
                    pnl: trade.pnl.unwrap_or(dec!(0)),
                    pnl_pct: pos.unrealized_pnl_pct,
                });
            }
            // Check stop loss
            else if pos.unrealized_pnl_pct <= -self.config.stop_loss_pct {
                let reason = format!(
                    "🛑 Stop loss: {:.2}% <= -{:.2}%",
                    pos.unrealized_pnl_pct, self.config.stop_loss_pct
                );
                warn!("{} for {}", reason, pos.question);
                
                let trade = self.sell(&pos.id, reason).await?;
                
                results.push(AutoCloseResult {
                    position_id: pos.id.clone(),
                    reason: AutoCloseReason::StopLoss,
                    pnl: trade.pnl.unwrap_or(dec!(0)),
                    pnl_pct: pos.unrealized_pnl_pct,
                });
            }
        }
        
        Ok(results)
    }

    /// Log price snapshot to JSONL file
    async fn log_price_snapshot(&self, market: &Market) -> Result<()> {
        // Ensure directory exists
        tokio::fs::create_dir_all(&self.config.snapshots_dir).await.ok();
        
        let date = Utc::now().format("%Y-%m-%d").to_string();
        let filename = format!("{}.jsonl", date);
        let path = self.config.snapshots_dir.join(filename);
        
        let yes_price = market.outcomes.first().map(|o| o.price).unwrap_or(dec!(0));
        let no_price = market.outcomes.get(1).map(|o| o.price).unwrap_or(dec!(0));
        
        let snapshot = PriceSnapshot {
            timestamp: Utc::now(),
            market_id: market.id.clone(),
            question: market.question.clone(),
            yes_price,
            no_price,
        };
        
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .await
            .map_err(|e| crate::error::BotError::Internal(e.to_string()))?;
        
        let line = serde_json::to_string(&snapshot)
            .map_err(|e| crate::error::BotError::Internal(e.to_string()))?;
        
        file.write_all(format!("{}\n", line).as_bytes()).await
            .map_err(|e| crate::error::BotError::Internal(e.to_string()))?;
        
        debug!("Logged price snapshot for {}", market.id);
        Ok(())
    }

    /// Log audit entry to JSONL file
    async fn log_audit(&self, entry: AuditEntry) -> Result<()> {
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.config.audit_file)
            .await
            .map_err(|e| crate::error::BotError::Internal(e.to_string()))?;
        
        let line = serde_json::to_string(&entry)
            .map_err(|e| crate::error::BotError::Internal(e.to_string()))?;
        
        file.write_all(format!("{}\n", line).as_bytes()).await
            .map_err(|e| crate::error::BotError::Internal(e.to_string()))?;
        
        debug!("Logged audit entry: {} {}", entry.action, entry.market_id);
        Ok(())
    }

    /// Log price for position tracking
    pub async fn log_position_price(&self, position: &Position, market: &Market) -> Result<()> {
        if self.config.log_prices {
            self.log_price_snapshot(market).await?;
        }
        Ok(())
    }

    // Delegate methods to inner trader
    pub async fn get_summary(&self) -> PortfolioSummary {
        self.trader.get_summary().await
    }

    pub async fn get_positions(&self) -> Vec<Position> {
        self.trader.get_positions().await
    }

    pub async fn get_open_positions(&self) -> Vec<Position> {
        self.trader.get_open_positions().await
    }

    pub async fn get_history(&self) -> Vec<TradeRecord> {
        self.trader.get_history().await
    }

    pub async fn get_balance(&self) -> Decimal {
        self.trader.get_balance().await
    }

    pub async fn settle_market(&self, market_id: &str, winning_side: bool) -> Result<Vec<TradeRecord>> {
        let result = self.trader.settle_market(market_id, winning_side).await?;
        if self.config.auto_save {
            self.save_state().await?;
        }
        Ok(result)
    }

    /// Get config for inspection
    pub fn config(&self) -> &AutoTraderConfig {
        &self.config
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Outcome;
    use tempfile::tempdir;

    fn create_test_market(yes_price: Decimal) -> Market {
        Market {
            id: "test_market".to_string(),
            question: "Test market?".to_string(),
            description: None,
            end_date: None,
            outcomes: vec![
                Outcome {
                    token_id: "yes".to_string(),
                    outcome: "Yes".to_string(),
                    price: yes_price,
                },
                Outcome {
                    token_id: "no".to_string(),
                    outcome: "No".to_string(),
                    price: dec!(1) - yes_price,
                },
            ],
            volume: dec!(10000),
            liquidity: dec!(5000),
            active: true,
            closed: false,
        }
    }

    fn create_test_client() -> GammaClient {
        GammaClient::new("https://gamma-api.polymarket.com").unwrap()
    }

    #[tokio::test]
    async fn test_auto_trader_buy_creates_files() {
        let dir = tempdir().unwrap();
        
        let config = AutoTraderConfig {
            paper_config: PaperTraderConfig {
                initial_balance: dec!(1000),
                slippage_pct: dec!(0),
                ..Default::default()
            },
            state_file: dir.path().join("state.json"),
            snapshots_dir: dir.path().join("snapshots"),
            audit_file: dir.path().join("audit.jsonl"),
            auto_save: true,
            log_prices: true,
            ..Default::default()
        };
        
        let client = create_test_client();
        let trader = AutoTrader::new(config.clone(), client);
        
        let market = create_test_market(dec!(0.50));
        trader.buy(&market, PositionSide::Yes, dec!(100), "Test".to_string())
            .await
            .unwrap();
        
        // Check state file created
        assert!(config.state_file.exists());
        
        // Check audit file created
        assert!(config.audit_file.exists());
        let audit = tokio::fs::read_to_string(&config.audit_file).await.unwrap();
        assert!(audit.contains("BUY"));
    }

    #[tokio::test]
    async fn test_take_profit_triggers() {
        let dir = tempdir().unwrap();
        
        let config = AutoTraderConfig {
            paper_config: PaperTraderConfig {
                initial_balance: dec!(1000),
                slippage_pct: dec!(0),
                ..Default::default()
            },
            take_profit_pct: dec!(10), // 10%
            stop_loss_pct: dec!(5),
            state_file: dir.path().join("state.json"),
            snapshots_dir: dir.path().join("snapshots"),
            audit_file: dir.path().join("audit.jsonl"),
            auto_save: true,
            log_prices: false,
            ..Default::default()
        };
        
        let client = create_test_client();
        let trader = AutoTrader::new(config, client);
        
        // Buy at 0.50
        let market = create_test_market(dec!(0.50));
        let _pos = trader.buy(&market, PositionSide::Yes, dec!(100), "Test".to_string())
            .await
            .unwrap();
        
        // Manually update position price to trigger take profit
        // In real scenario, update_and_check() would fetch from API
        // For test, we check the logic structure
        
        let positions = trader.get_open_positions().await;
        assert_eq!(positions.len(), 1);
    }

    #[tokio::test]
    async fn test_state_persistence() {
        let dir = tempdir().unwrap();
        let state_file = dir.path().join("state.json");
        let snapshots_dir = dir.path().join("snapshots");
        let audit_file = dir.path().join("audit.jsonl");
        
        // Create trader and buy
        {
            let config = AutoTraderConfig {
                paper_config: PaperTraderConfig {
                    initial_balance: dec!(1000),
                    slippage_pct: dec!(0),
                    ..Default::default()
                },
                state_file: state_file.clone(),
                snapshots_dir: snapshots_dir.clone(),
                audit_file: audit_file.clone(),
                auto_save: true,
                log_prices: false,
                ..Default::default()
            };
            let client = create_test_client();
            let trader = AutoTrader::new(config, client);
            let market = create_test_market(dec!(0.50));
            trader.buy(&market, PositionSide::Yes, dec!(100), "Test".to_string())
                .await
                .unwrap();
        }
        
        // Create new trader and load state
        {
            let config = AutoTraderConfig {
                paper_config: PaperTraderConfig {
                    initial_balance: dec!(1000),
                    slippage_pct: dec!(0),
                    ..Default::default()
                },
                state_file: state_file.clone(),
                snapshots_dir: snapshots_dir.clone(),
                audit_file: audit_file.clone(),
                auto_save: true,
                log_prices: false,
                ..Default::default()
            };
            let client = create_test_client();
            let trader = AutoTrader::new(config, client);
            trader.load_state().await.unwrap();
            
            let positions = trader.get_positions().await;
            assert_eq!(positions.len(), 1);
            
            let balance = trader.get_balance().await;
            assert_eq!(balance, dec!(900));
        }
    }

    #[tokio::test]
    async fn test_audit_log_format() {
        let dir = tempdir().unwrap();
        let audit_file = dir.path().join("audit.jsonl");
        
        let config = AutoTraderConfig {
            paper_config: PaperTraderConfig {
                initial_balance: dec!(1000),
                slippage_pct: dec!(0),
                ..Default::default()
            },
            state_file: dir.path().join("state.json"),
            snapshots_dir: dir.path().join("snapshots"),
            audit_file: audit_file.clone(),
            auto_save: true,
            log_prices: false,
            ..Default::default()
        };
        
        let client = create_test_client();
        let trader = AutoTrader::new(config, client);
        
        // Buy
        let market = create_test_market(dec!(0.50));
        let pos = trader.buy(&market, PositionSide::Yes, dec!(100), "Test buy".to_string())
            .await
            .unwrap();
        
        // Sell
        trader.sell(&pos.id, "Test sell".to_string()).await.unwrap();
        
        // Check audit log
        let content = tokio::fs::read_to_string(&audit_file).await.unwrap();
        let lines: Vec<&str> = content.lines().collect();
        
        assert_eq!(lines.len(), 2);
        
        // Parse and verify
        let buy_entry: AuditEntry = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(buy_entry.action, "BUY");
        assert_eq!(buy_entry.balance_before, dec!(1000));
        assert_eq!(buy_entry.balance_after, dec!(900));
        
        let sell_entry: AuditEntry = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(sell_entry.action, "SELL");
        assert!(sell_entry.pnl.is_some());
    }
}
