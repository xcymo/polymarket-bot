//! Real-time monitoring dashboard API
//!
//! Provides HTTP endpoints for live monitoring of bot performance,
//! market state, and risk metrics.

use axum::{
    extract::State,
    http::StatusCode,
    response::Json,
    routing::get,
    Router,
};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;
use chrono::{DateTime, Utc, Duration};

/// Dashboard state shared across handlers
pub struct DashboardState {
    pub metrics: RwLock<DashboardMetrics>,
    pub trades: RwLock<Vec<TradeEntry>>,
    pub positions: RwLock<Vec<PositionEntry>>,
    pub alerts: RwLock<Vec<AlertEntry>>,
}

/// Core metrics displayed on dashboard
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DashboardMetrics {
    /// Current portfolio value in USDC
    pub portfolio_value: Decimal,
    /// Starting capital
    pub initial_capital: Decimal,
    /// Unrealized P&L
    pub unrealized_pnl: Decimal,
    /// Realized P&L
    pub realized_pnl: Decimal,
    /// Total P&L (realized + unrealized)
    pub total_pnl: Decimal,
    /// Return percentage
    pub return_pct: Decimal,
    /// Win rate (0-100)
    pub win_rate: Decimal,
    /// Total number of trades
    pub total_trades: u64,
    /// Winning trades
    pub winning_trades: u64,
    /// Losing trades
    pub losing_trades: u64,
    /// Sharpe ratio (annualized)
    pub sharpe_ratio: Decimal,
    /// Maximum drawdown percentage
    pub max_drawdown_pct: Decimal,
    /// Current drawdown percentage
    pub current_drawdown_pct: Decimal,
    /// Profit factor (gross profit / gross loss)
    pub profit_factor: Decimal,
    /// Average trade duration in minutes
    pub avg_trade_duration_mins: u64,
    /// Number of active positions
    pub active_positions: u64,
    /// Available buying power
    pub buying_power: Decimal,
    /// Bot uptime in seconds
    pub uptime_secs: u64,
    /// Last update timestamp
    pub last_updated: DateTime<Utc>,
}

/// Individual trade record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeEntry {
    pub id: String,
    pub timestamp: DateTime<Utc>,
    pub market_id: String,
    pub market_name: String,
    pub side: TradeSide,
    pub outcome: String,
    pub size: Decimal,
    pub price: Decimal,
    pub fees: Decimal,
    pub pnl: Option<Decimal>,
    pub status: TradeStatus,
    pub strategy: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum TradeSide {
    Buy,
    Sell,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum TradeStatus {
    Pending,
    Filled,
    PartiallyFilled,
    Cancelled,
    Failed,
}

/// Current position
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PositionEntry {
    pub market_id: String,
    pub market_name: String,
    pub outcome: String,
    pub side: TradeSide,
    pub size: Decimal,
    pub avg_entry_price: Decimal,
    pub current_price: Decimal,
    pub unrealized_pnl: Decimal,
    pub unrealized_pnl_pct: Decimal,
    pub opened_at: DateTime<Utc>,
    pub strategy: String,
}

/// Alert entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertEntry {
    pub id: String,
    pub timestamp: DateTime<Utc>,
    pub severity: AlertSeverity,
    pub category: String,
    pub message: String,
    pub acknowledged: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum AlertSeverity {
    Info,
    Warning,
    Critical,
}

impl DashboardState {
    /// Create new dashboard state
    pub fn new(initial_capital: Decimal) -> Self {
        let mut metrics = DashboardMetrics::default();
        metrics.initial_capital = initial_capital;
        metrics.portfolio_value = initial_capital;
        metrics.buying_power = initial_capital;
        metrics.last_updated = Utc::now();
        
        Self {
            metrics: RwLock::new(metrics),
            trades: RwLock::new(Vec::new()),
            positions: RwLock::new(Vec::new()),
            alerts: RwLock::new(Vec::new()),
        }
    }
    
    /// Record a new trade
    pub async fn record_trade(&self, trade: TradeEntry) {
        let mut trades = self.trades.write().await;
        trades.push(trade.clone());
        
        // Keep only last 1000 trades
        if trades.len() > 1000 {
            trades.remove(0);
        }
        
        // Update metrics
        if let Some(pnl) = trade.pnl {
            let mut metrics = self.metrics.write().await;
            metrics.total_trades += 1;
            metrics.realized_pnl += pnl;
            
            if pnl > Decimal::ZERO {
                metrics.winning_trades += 1;
            } else if pnl < Decimal::ZERO {
                metrics.losing_trades += 1;
            }
            
            metrics.win_rate = if metrics.total_trades > 0 {
                Decimal::from(metrics.winning_trades) / Decimal::from(metrics.total_trades) * Decimal::ONE_HUNDRED
            } else {
                Decimal::ZERO
            };
            
            metrics.last_updated = Utc::now();
        }
    }
    
    /// Update position
    pub async fn update_position(&self, position: PositionEntry) {
        let mut positions = self.positions.write().await;
        
        // Find and update or add
        if let Some(existing) = positions.iter_mut().find(|p| {
            p.market_id == position.market_id && p.outcome == position.outcome
        }) {
            *existing = position;
        } else {
            positions.push(position);
        }
        
        // Recalculate unrealized P&L
        let total_unrealized: Decimal = positions.iter()
            .map(|p| p.unrealized_pnl)
            .sum();
        
        let mut metrics = self.metrics.write().await;
        metrics.unrealized_pnl = total_unrealized;
        metrics.total_pnl = metrics.realized_pnl + metrics.unrealized_pnl;
        metrics.active_positions = positions.len() as u64;
        
        // Calculate return percentage
        if metrics.initial_capital > Decimal::ZERO {
            metrics.return_pct = metrics.total_pnl / metrics.initial_capital * Decimal::ONE_HUNDRED;
        }
        
        metrics.last_updated = Utc::now();
    }
    
    /// Remove closed position
    pub async fn close_position(&self, market_id: &str, outcome: &str) {
        let mut positions = self.positions.write().await;
        positions.retain(|p| !(p.market_id == market_id && p.outcome == outcome));
        
        let mut metrics = self.metrics.write().await;
        metrics.active_positions = positions.len() as u64;
        metrics.last_updated = Utc::now();
    }
    
    /// Add alert
    pub async fn add_alert(&self, alert: AlertEntry) {
        let mut alerts = self.alerts.write().await;
        alerts.insert(0, alert);
        
        // Keep only last 100 alerts
        if alerts.len() > 100 {
            alerts.truncate(100);
        }
    }
    
    /// Update portfolio value and calculate drawdown
    pub async fn update_portfolio_value(&self, value: Decimal) {
        let mut metrics = self.metrics.write().await;
        let peak = metrics.portfolio_value.max(value);
        
        metrics.portfolio_value = value;
        
        // Calculate drawdown
        if peak > Decimal::ZERO {
            let drawdown = (peak - value) / peak * Decimal::ONE_HUNDRED;
            metrics.current_drawdown_pct = drawdown;
            
            if drawdown > metrics.max_drawdown_pct {
                metrics.max_drawdown_pct = drawdown;
            }
        }
        
        metrics.last_updated = Utc::now();
    }
    
    /// Calculate Sharpe ratio from trade history
    pub async fn calculate_sharpe(&self, risk_free_rate: Decimal) {
        let trades = self.trades.read().await;
        
        if trades.len() < 2 {
            return;
        }
        
        // Get returns
        let returns: Vec<Decimal> = trades.iter()
            .filter_map(|t| t.pnl)
            .collect();
        
        if returns.is_empty() {
            return;
        }
        
        // Calculate mean return
        let sum: Decimal = returns.iter().sum();
        let mean = sum / Decimal::from(returns.len());
        
        // Calculate standard deviation
        let variance: Decimal = returns.iter()
            .map(|r| {
                let diff = *r - mean;
                diff * diff
            })
            .sum::<Decimal>() / Decimal::from(returns.len());
        
        // Approximate sqrt for std dev (Newton's method)
        let std_dev = decimal_sqrt(variance);
        
        if std_dev > Decimal::ZERO {
            let sharpe = (mean - risk_free_rate) / std_dev;
            // Annualize (assuming hourly trades, ~8760 hours/year)
            let annualized = sharpe * decimal_sqrt(Decimal::from(8760));
            
            let mut metrics = self.metrics.write().await;
            metrics.sharpe_ratio = annualized;
        }
    }
    
    /// Calculate profit factor
    pub async fn calculate_profit_factor(&self) {
        let trades = self.trades.read().await;
        
        let mut gross_profit = Decimal::ZERO;
        let mut gross_loss = Decimal::ZERO;
        
        for trade in trades.iter() {
            if let Some(pnl) = trade.pnl {
                if pnl > Decimal::ZERO {
                    gross_profit += pnl;
                } else {
                    gross_loss += pnl.abs();
                }
            }
        }
        
        let mut metrics = self.metrics.write().await;
        metrics.profit_factor = if gross_loss > Decimal::ZERO {
            gross_profit / gross_loss
        } else if gross_profit > Decimal::ZERO {
            Decimal::from(999) // Infinite profit factor capped
        } else {
            Decimal::ZERO
        };
    }
}

/// Newton's method square root for Decimal
fn decimal_sqrt(n: Decimal) -> Decimal {
    if n <= Decimal::ZERO {
        return Decimal::ZERO;
    }
    
    let mut x = n;
    let two = Decimal::from(2);
    
    // 10 iterations should be enough for convergence
    for _ in 0..10 {
        let x_next = (x + n / x) / two;
        if (x_next - x).abs() < Decimal::new(1, 10) {
            return x_next;
        }
        x = x_next;
    }
    
    x
}

// ============ HTTP API Handlers ============

/// Get current metrics
async fn get_metrics(
    State(state): State<Arc<DashboardState>>,
) -> Json<DashboardMetrics> {
    let metrics = state.metrics.read().await;
    Json(metrics.clone())
}

/// Get recent trades
async fn get_trades(
    State(state): State<Arc<DashboardState>>,
) -> Json<Vec<TradeEntry>> {
    let trades = state.trades.read().await;
    Json(trades.clone())
}

/// Get current positions
async fn get_positions(
    State(state): State<Arc<DashboardState>>,
) -> Json<Vec<PositionEntry>> {
    let positions = state.positions.read().await;
    Json(positions.clone())
}

/// Get alerts
async fn get_alerts(
    State(state): State<Arc<DashboardState>>,
) -> Json<Vec<AlertEntry>> {
    let alerts = state.alerts.read().await;
    Json(alerts.clone())
}

/// Health check
async fn health_check() -> &'static str {
    "OK"
}

/// Dashboard summary response
#[derive(Serialize)]
struct DashboardSummary {
    metrics: DashboardMetrics,
    recent_trades: Vec<TradeEntry>,
    positions: Vec<PositionEntry>,
    active_alerts: Vec<AlertEntry>,
}

/// Get full dashboard summary
async fn get_summary(
    State(state): State<Arc<DashboardState>>,
) -> Json<DashboardSummary> {
    let metrics = state.metrics.read().await.clone();
    let trades = state.trades.read().await;
    let positions = state.positions.read().await.clone();
    let alerts = state.alerts.read().await;
    
    // Get last 10 trades
    let recent_trades: Vec<TradeEntry> = trades.iter()
        .rev()
        .take(10)
        .cloned()
        .collect();
    
    // Get unacknowledged alerts
    let active_alerts: Vec<AlertEntry> = alerts.iter()
        .filter(|a| !a.acknowledged)
        .take(5)
        .cloned()
        .collect();
    
    Json(DashboardSummary {
        metrics,
        recent_trades,
        positions,
        active_alerts,
    })
}

/// Create dashboard router
pub fn create_router(state: Arc<DashboardState>) -> Router {
    Router::new()
        .route("/health", get(health_check))
        .route("/metrics", get(get_metrics))
        .route("/trades", get(get_trades))
        .route("/positions", get(get_positions))
        .route("/alerts", get(get_alerts))
        .route("/summary", get(get_summary))
        .with_state(state)
}

/// Start dashboard server
pub async fn start_dashboard(
    state: Arc<DashboardState>,
    port: u16,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let app = create_router(state);
    
    let addr = std::net::SocketAddr::from(([0, 0, 0, 0], port));
    tracing::info!("Dashboard server starting on http://{}", addr);
    
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_dashboard_state_creation() {
        let state = DashboardState::new(Decimal::from(1000));
        let metrics = state.metrics.read().await;
        
        assert_eq!(metrics.initial_capital, Decimal::from(1000));
        assert_eq!(metrics.portfolio_value, Decimal::from(1000));
        assert_eq!(metrics.total_trades, 0);
    }

    #[tokio::test]
    async fn test_record_trade_updates_metrics() {
        let state = DashboardState::new(Decimal::from(1000));
        
        let trade = TradeEntry {
            id: "t1".to_string(),
            timestamp: Utc::now(),
            market_id: "m1".to_string(),
            market_name: "BTC Up/Down".to_string(),
            side: TradeSide::Buy,
            outcome: "Up".to_string(),
            size: Decimal::from(100),
            price: Decimal::new(55, 2), // 0.55
            fees: Decimal::new(5, 1), // 0.5
            pnl: Some(Decimal::from(15)),
            status: TradeStatus::Filled,
            strategy: "RSI".to_string(),
        };
        
        state.record_trade(trade).await;
        
        let metrics = state.metrics.read().await;
        assert_eq!(metrics.total_trades, 1);
        assert_eq!(metrics.winning_trades, 1);
        assert_eq!(metrics.realized_pnl, Decimal::from(15));
    }

    #[tokio::test]
    async fn test_win_rate_calculation() {
        let state = DashboardState::new(Decimal::from(1000));
        
        // Record winning trade
        state.record_trade(TradeEntry {
            id: "t1".to_string(),
            timestamp: Utc::now(),
            market_id: "m1".to_string(),
            market_name: "Test".to_string(),
            side: TradeSide::Buy,
            outcome: "Up".to_string(),
            size: Decimal::from(100),
            price: Decimal::new(5, 1),
            fees: Decimal::ZERO,
            pnl: Some(Decimal::from(10)),
            status: TradeStatus::Filled,
            strategy: "test".to_string(),
        }).await;
        
        // Record losing trade
        state.record_trade(TradeEntry {
            id: "t2".to_string(),
            timestamp: Utc::now(),
            market_id: "m2".to_string(),
            market_name: "Test".to_string(),
            side: TradeSide::Buy,
            outcome: "Down".to_string(),
            size: Decimal::from(100),
            price: Decimal::new(5, 1),
            fees: Decimal::ZERO,
            pnl: Some(Decimal::from(-5)),
            status: TradeStatus::Filled,
            strategy: "test".to_string(),
        }).await;
        
        let metrics = state.metrics.read().await;
        assert_eq!(metrics.total_trades, 2);
        assert_eq!(metrics.win_rate, Decimal::from(50)); // 50%
    }

    #[tokio::test]
    async fn test_drawdown_calculation() {
        let state = DashboardState::new(Decimal::from(1000));
        
        // Portfolio goes up
        state.update_portfolio_value(Decimal::from(1200)).await;
        
        // Portfolio drops
        state.update_portfolio_value(Decimal::from(1000)).await;
        
        let metrics = state.metrics.read().await;
        // Drawdown should be (1200-1000)/1200 * 100 = 16.67%
        assert!(metrics.max_drawdown_pct > Decimal::from(16));
        assert!(metrics.max_drawdown_pct < Decimal::from(17));
    }

    #[tokio::test]
    async fn test_position_update() {
        let state = DashboardState::new(Decimal::from(1000));
        
        let position = PositionEntry {
            market_id: "m1".to_string(),
            market_name: "BTC Up/Down".to_string(),
            outcome: "Up".to_string(),
            side: TradeSide::Buy,
            size: Decimal::from(100),
            avg_entry_price: Decimal::new(5, 1),
            current_price: Decimal::new(6, 1),
            unrealized_pnl: Decimal::from(10),
            unrealized_pnl_pct: Decimal::from(20),
            opened_at: Utc::now(),
            strategy: "RSI".to_string(),
        };
        
        state.update_position(position).await;
        
        let metrics = state.metrics.read().await;
        assert_eq!(metrics.active_positions, 1);
        assert_eq!(metrics.unrealized_pnl, Decimal::from(10));
    }

    #[tokio::test]
    async fn test_alert_management() {
        let state = DashboardState::new(Decimal::from(1000));
        
        let alert = AlertEntry {
            id: "a1".to_string(),
            timestamp: Utc::now(),
            severity: AlertSeverity::Warning,
            category: "risk".to_string(),
            message: "Max drawdown approaching".to_string(),
            acknowledged: false,
        };
        
        state.add_alert(alert).await;
        
        let alerts = state.alerts.read().await;
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].severity, AlertSeverity::Warning);
    }

    #[tokio::test]
    async fn test_profit_factor() {
        let state = DashboardState::new(Decimal::from(1000));
        
        // Two wins of 20 each
        for i in 0..2 {
            state.record_trade(TradeEntry {
                id: format!("w{}", i),
                timestamp: Utc::now(),
                market_id: "m1".to_string(),
                market_name: "Test".to_string(),
                side: TradeSide::Buy,
                outcome: "Up".to_string(),
                size: Decimal::from(100),
                price: Decimal::new(5, 1),
                fees: Decimal::ZERO,
                pnl: Some(Decimal::from(20)),
                status: TradeStatus::Filled,
                strategy: "test".to_string(),
            }).await;
        }
        
        // One loss of 10
        state.record_trade(TradeEntry {
            id: "l1".to_string(),
            timestamp: Utc::now(),
            market_id: "m2".to_string(),
            market_name: "Test".to_string(),
            side: TradeSide::Buy,
            outcome: "Down".to_string(),
            size: Decimal::from(100),
            price: Decimal::new(5, 1),
            fees: Decimal::ZERO,
            pnl: Some(Decimal::from(-10)),
            status: TradeStatus::Filled,
            strategy: "test".to_string(),
        }).await;
        
        state.calculate_profit_factor().await;
        
        let metrics = state.metrics.read().await;
        // Profit factor should be 40/10 = 4.0
        assert_eq!(metrics.profit_factor, Decimal::from(4));
    }

    #[test]
    fn test_decimal_sqrt() {
        let result = decimal_sqrt(Decimal::from(4));
        assert!((result - Decimal::from(2)).abs() < Decimal::new(1, 6));
        
        let result = decimal_sqrt(Decimal::from(9));
        assert!((result - Decimal::from(3)).abs() < Decimal::new(1, 6));
        
        let result = decimal_sqrt(Decimal::ZERO);
        assert_eq!(result, Decimal::ZERO);
    }

    #[tokio::test]
    async fn test_sharpe_ratio_calculation() {
        let state = DashboardState::new(Decimal::from(1000));
        
        // Add several trades with returns
        let returns = vec![10, 5, -3, 8, 12, -2, 7, 15, -1, 6];
        for (i, r) in returns.iter().enumerate() {
            state.record_trade(TradeEntry {
                id: format!("t{}", i),
                timestamp: Utc::now(),
                market_id: "m1".to_string(),
                market_name: "Test".to_string(),
                side: TradeSide::Buy,
                outcome: "Up".to_string(),
                size: Decimal::from(100),
                price: Decimal::new(5, 1),
                fees: Decimal::ZERO,
                pnl: Some(Decimal::from(*r)),
                status: TradeStatus::Filled,
                strategy: "test".to_string(),
            }).await;
        }
        
        state.calculate_sharpe(Decimal::ZERO).await;
        
        let metrics = state.metrics.read().await;
        // Should have a positive Sharpe ratio given mostly positive returns
        assert!(metrics.sharpe_ratio > Decimal::ZERO);
    }

    #[tokio::test]
    async fn test_close_position() {
        let state = DashboardState::new(Decimal::from(1000));
        
        let position = PositionEntry {
            market_id: "m1".to_string(),
            market_name: "Test".to_string(),
            outcome: "Up".to_string(),
            side: TradeSide::Buy,
            size: Decimal::from(100),
            avg_entry_price: Decimal::new(5, 1),
            current_price: Decimal::new(6, 1),
            unrealized_pnl: Decimal::from(10),
            unrealized_pnl_pct: Decimal::from(20),
            opened_at: Utc::now(),
            strategy: "test".to_string(),
        };
        
        state.update_position(position).await;
        assert_eq!(state.positions.read().await.len(), 1);
        
        state.close_position("m1", "Up").await;
        assert_eq!(state.positions.read().await.len(), 0);
    }
}
