//! Cross-Price Arbitrage Scanner CLI
//!
//! Implements the target trader (k9Q2mX4L8A7ZP3R) strategy:
//! Buy Up + Down when total < $1, lock in guaranteed profit
//!
//! Usage:
//!   cross_price_arb scan         - One-time scan for opportunities
//!   cross_price_arb paper        - Run paper trading simulation
//!   cross_price_arb paper --live - Run until stopped

use chrono::Utc;
use polymarket_bot::scanner::{
    CrossPriceScanner, CrossPriceConfig, CrossPricePaperTrader,
};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use std::env;
use std::time::Duration;
use tokio::time::sleep;
use tracing::{info, Level};
use tracing_subscriber::FmtSubscriber;

const GAMMA_URL: &str = "https://gamma-api.polymarket.com";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize logging
    let subscriber = FmtSubscriber::builder()
        .with_max_level(Level::INFO)
        .with_target(false)
        .with_thread_ids(false)
        .finish();
    tracing::subscriber::set_global_default(subscriber)?;

    let args: Vec<String> = env::args().collect();
    let command = args.get(1).map(|s| s.as_str()).unwrap_or("scan");

    match command {
        "scan" => run_scan().await,
        "paper" => {
            let live = args.iter().any(|a| a == "--live" || a == "-l");
            run_paper(live).await
        }
        _ => {
            println!("Cross-Price Arbitrage Scanner");
            println!();
            println!("Commands:");
            println!("  scan   - One-time scan for opportunities");
            println!("  paper  - Run paper trading simulation (add --live for continuous)");
            println!();
            println!("Example:");
            println!("  cross_price_arb scan");
            println!("  cross_price_arb paper --live");
            Ok(())
        }
    }
}

async fn run_scan() -> anyhow::Result<()> {
    info!("🔍 Scanning for cross-price arbitrage opportunities...");

    let config = CrossPriceConfig {
        min_spread: dec!(0.005),        // 0.5% minimum (lowered for more opportunities)
        max_spread: dec!(0.15),         // 15% max
        min_time_remaining: 30,         // At least 30 seconds
        max_time_remaining: 900,        // Within 15 minutes
        max_position: dec!(100),
        fee_rate: Decimal::ZERO,
    };

    let mut scanner = CrossPriceScanner::new(GAMMA_URL, config);
    
    match scanner.scan_all().await {
        Ok(opps) => {
            if opps.is_empty() {
                info!("No opportunities found at this time.");
                info!("This is normal - spreads are usually tight (< 0.5%)");
            } else {
                println!("\n🎯 Found {} opportunities:\n", opps.len());
                for opp in &opps {
                    println!(
                        "{} | Up: {:.1}¢ + Down: {:.1}¢ = {:.1}¢ | Spread: {:.2}% | {}s left",
                        opp.symbol,
                        opp.up_price * dec!(100),
                        opp.down_price * dec!(100),
                        opp.total_cost * dec!(100),
                        opp.spread * dec!(100),
                        opp.seconds_remaining
                    );
                    println!("   Expected profit on $100: ${:.2}", opp.expected_profit_usd);
                    println!();
                }
            }
        }
        Err(e) => {
            info!("Scan error: {}", e);
        }
    }

    println!("{}", scanner.summary());
    Ok(())
}

async fn run_paper(live: bool) -> anyhow::Result<()> {
    info!("💰 Starting cross-price arbitrage paper trading...");
    info!("Initial balance: $1000");
    info!("Mode: {}", if live { "LIVE (continuous)" } else { "Single scan" });

    let config = CrossPriceConfig {
        min_spread: dec!(0.01),         // 1% minimum for paper trading
        max_spread: dec!(0.10),         // 10% max
        min_time_remaining: 60,         // At least 1 minute
        max_time_remaining: 600,        // Within 10 minutes
        max_position: dec!(50),         // $50 per trade
        fee_rate: Decimal::ZERO,
    };

    let mut scanner = CrossPriceScanner::new(GAMMA_URL, config);
    let mut trader = CrossPricePaperTrader::new(dec!(1000));

    let scan_interval = Duration::from_secs(30);
    let mut last_scan = Utc::now() - chrono::Duration::seconds(60);

    loop {
        // Settle any expired positions
        trader.settle();

        // Scan for new opportunities
        let now = Utc::now();
        if (now - last_scan).num_seconds() >= 30 {
            last_scan = now;
            
            match scanner.scan_all().await {
                Ok(opps) => {
                    for opp in opps {
                        // Check if we can afford to enter
                        let position_size = dec!(50).min(trader.balance() * dec!(0.2));
                        if position_size >= dec!(10) && opp.is_valid() {
                            if trader.enter(&opp, position_size) {
                                scanner.record_trade(position_size, opp.calculate_profit(position_size), true);
                            }
                        }
                    }
                }
                Err(e) => {
                    info!("Scan error: {}", e);
                }
            }
        }

        // Print status
        println!("\n{}", trader.summary());
        println!("{}", scanner.summary());

        if !live {
            break;
        }

        info!("Sleeping {}s until next scan...", scan_interval.as_secs());
        sleep(scan_interval).await;
    }

    Ok(())
}
