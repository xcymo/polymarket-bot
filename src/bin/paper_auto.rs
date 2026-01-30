//! Automated Paper Trading - runs continuously like Python bot

use polymarket_bot::client::GammaClient;
use polymarket_bot::paper::{PaperTrader, PaperTraderConfig, PositionSide};
use polymarket_bot::types::Market;
use rust_decimal_macros::dec;
use std::time::Duration;
use tokio::time::sleep;
use tracing::{info, warn, error};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    
    let config = PaperTraderConfig {
        initial_balance: dec!(1000),
        max_position_pct: dec!(10),  // 10% max per position
        slippage_pct: dec!(0.25),    // 0.25% slippage
        fee_pct: dec!(0.1),          // 0.1% fee
        save_interval: 60,
        state_file: Some("paper_auto_state.json".to_string()),
    };
    
    let gamma = GammaClient::new("https://gamma-api.polymarket.com")?;
    let trader = PaperTrader::new(config, gamma.clone());
    
    info!("🤖 Paper Trading Bot Started");
    info!("Initial balance: $1000");
    
    loop {
        if let Err(e) = run_cycle(&trader, &gamma).await {
            error!("Cycle error: {}", e);
        }
        
        // Wait 5 minutes between cycles
        sleep(Duration::from_secs(300)).await;
    }
}

async fn run_cycle(trader: &PaperTrader, gamma: &GammaClient) -> anyhow::Result<()> {
    info!("📊 Scanning markets...");
    
    // Get top markets by volume
    let markets = gamma.get_top_markets(50).await?;
    
    for market in markets.iter().take(20) {
        // Skip if already have position
        let positions = trader.get_positions().await;
        if positions.iter().any(|p| p.market_id == market.id) {
            continue;
        }
        
        // Get YES price
        let yes_price = market.outcomes.iter()
            .find(|o| o.outcome.to_lowercase() == "yes")
            .map(|o| o.price)
            .unwrap_or(dec!(0.5));
            
        let price_f64 = yes_price.to_string().parse::<f64>().unwrap_or(0.5);
        
        // Buy signal: price < 0.15 (potential upside)
        if price_f64 < 0.15 && price_f64 > 0.02 {
            let amount = dec!(25);
            
            match trader.buy(market, PositionSide::Yes, amount, 
                format!("Low price: {:.1}%", price_f64 * 100.0)).await 
            {
                Ok(_) => {
                    info!("✅ BUY YES {} @ {:.2}¢", 
                        &market.question.chars().take(30).collect::<String>(), 
                        price_f64 * 100.0);
                }
                Err(e) => warn!("Buy failed: {}", e),
            }
        }
        
        // Sell signal: YES price > 0.85, bet NO
        if price_f64 > 0.85 && price_f64 < 0.98 {
            let amount = dec!(25);
            match trader.buy(market, PositionSide::No, amount,
                format!("High YES: {:.1}%", price_f64 * 100.0)).await
            {
                Ok(_) => {
                    info!("✅ BUY NO {} @ {:.2}¢", 
                        &market.question.chars().take(30).collect::<String>(),
                        (1.0 - price_f64) * 100.0);
                }
                Err(e) => warn!("Buy NO failed: {}", e),
            }
        }
    }
    
    // Check existing positions for exit
    check_exits(trader, gamma).await?;
    
    // Check crypto 15m markets
    if let Err(e) = check_crypto_15m(trader).await {
        warn!("Crypto 15m check failed: {}", e);
    }
    
    // Print status
    let summary = trader.get_summary().await;
    info!("💰 ${:.2} | P&L: ${:.2} ({:.1}%) | Pos: {}", 
        summary.total_value,
        summary.total_pnl,
        summary.roi_percent,
        summary.open_positions
    );
    
    Ok(())
}

async fn check_exits(trader: &PaperTrader, gamma: &GammaClient) -> anyhow::Result<()> {
    let positions = trader.get_positions().await;
    
    for pos in positions {
        if let Ok(market) = gamma.get_market(&pos.market_id).await {
            let side_name = match pos.side {
                PositionSide::Yes => "yes",
                PositionSide::No => "no",
            };
            
            let current_price = market.outcomes.iter()
                .find(|o| o.outcome.to_lowercase() == side_name)
                .map(|o| o.price)
                .unwrap_or(pos.entry_price);
            
            let pnl_pct = if pos.entry_price > dec!(0) {
                let diff = current_price - pos.entry_price;
                (diff / pos.entry_price * dec!(100)).to_string()
                    .parse::<f64>().unwrap_or(0.0)
            } else {
                0.0
            };
            
            // Take profit +20% or stop loss -15%
            if pnl_pct > 20.0 || pnl_pct < -15.0 {
                let reason = if pnl_pct > 0.0 { 
                    format!("Take profit: {:.1}%", pnl_pct) 
                } else { 
                    format!("Stop loss: {:.1}%", pnl_pct) 
                };
                match trader.sell(&pos.id, reason).await {
                    Ok(_) => info!("📤 SOLD {} PnL: {:.1}%", 
                        &market.question.chars().take(30).collect::<String>(), pnl_pct),
                    Err(e) => warn!("Sell failed: {}", e),
                }
            }
        }
    }
    
    Ok(())
}

// Crypto 15m market auto-trading
async fn check_crypto_15m(trader: &PaperTrader) -> anyhow::Result<()> {
    let client = reqwest::Client::new();
    
    // Get current 15-min slot
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs();
    let slot = now - (now % 900);
    
    let url = format!("https://gamma-api.polymarket.com/events?slug=btc-updown-15m-{}", slot);
    let resp: serde_json::Value = client.get(&url).send().await?.json().await?;
    
    if let Some(event) = resp.as_array().and_then(|a| a.first()) {
        let title = event["title"].as_str().unwrap_or("BTC 15m");
        
        if let Some(market) = event["markets"].as_array().and_then(|m| m.first()) {
            let condition_id = market["conditionId"].as_str().unwrap_or("");
            
            // Check if already have position in this market
            let positions = trader.get_positions().await;
            if positions.iter().any(|p| p.market_id.contains(&slot.to_string())) {
                return Ok(()); // Already positioned
            }
            
            if let Some(prices) = market["outcomePrices"].as_str() {
                let prices: Vec<&str> = prices.trim_matches(|c| c == '[' || c == ']' || c == '"')
                    .split("\", \"").collect();
                if prices.len() >= 2 {
                    let up_price: f64 = prices[0].parse().unwrap_or(0.5);
                    let down_price: f64 = prices[1].parse().unwrap_or(0.5);
                    
                    info!("📈 BTC 15m: UP {:.1}% | DOWN {:.1}%", up_price * 100.0, down_price * 100.0);
                    
                    // Strategy: Buy contrarian when price < 25%
                    // Higher conviction = larger position
                    if up_price < 0.25 && up_price > 0.02 {
                        let amount = if up_price < 0.10 { dec!(50) } else { dec!(30) };
                        
                        // Create a mock market for the trader
                        let mock_market = polymarket_bot::types::Market {
                            id: format!("btc-15m-{}-up", slot),
                            question: format!("{} - UP", title),
                            outcomes: vec![
                                polymarket_bot::types::Outcome {
                                    token_id: condition_id.to_string(),
                                    outcome: "Yes".to_string(),
                                    price: rust_decimal::Decimal::from_f64_retain(up_price).unwrap_or(dec!(0.1)),
                                },
                            ],
                            volume: dec!(0),
                            liquidity: dec!(0),
                            end_date: None,
                            description: None, active: true,
                            closed: false,
                        };
                        
                        match trader.buy(&mock_market, PositionSide::Yes, amount,
                            format!("BTC 15m contrarian: UP at {:.1}%", up_price * 100.0)).await
                        {
                            Ok(_) => info!("🎰 BUY BTC UP @ {:.1}% - ${}", up_price * 100.0, amount),
                            Err(e) => warn!("BTC UP buy failed: {}", e),
                        }
                    } else if down_price < 0.25 && down_price > 0.02 {
                        let amount = if down_price < 0.10 { dec!(50) } else { dec!(30) };
                        
                        let mock_market = polymarket_bot::types::Market {
                            id: format!("btc-15m-{}-down", slot),
                            question: format!("{} - DOWN", title),
                            outcomes: vec![
                                polymarket_bot::types::Outcome {
                                    token_id: condition_id.to_string(),
                                    outcome: "Yes".to_string(),
                                    price: rust_decimal::Decimal::from_f64_retain(down_price).unwrap_or(dec!(0.1)),
                                },
                            ],
                            volume: dec!(0),
                            liquidity: dec!(0),
                            end_date: None,
                            description: None, active: true,
                            closed: false,
                        };
                        
                        match trader.buy(&mock_market, PositionSide::Yes, amount,
                            format!("BTC 15m contrarian: DOWN at {:.1}%", down_price * 100.0)).await
                        {
                            Ok(_) => info!("🎰 BUY BTC DOWN @ {:.1}% - ${}", down_price * 100.0, amount),
                            Err(e) => warn!("BTC DOWN buy failed: {}", e),
                        }
                    }
                }
            }
        }
    }
    
    Ok(())
}
