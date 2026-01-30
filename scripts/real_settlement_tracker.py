#!/usr/bin/env python3
"""
Real Settlement Tracker - NO fake wins/losses
1. Use real market data to decide trades
2. Record positions without simulating outcome
3. Check Polymarket API for REAL settlement
4. Calculate actual PnL only when market settles
"""

import requests
import json
import time
import os
import sys
from datetime import datetime, timezone, timedelta
from typing import Optional, List, Dict

sys.stdout.reconfigure(line_buffering=True)

# Configuration
INITIAL_CAPITAL = 100.0
MIN_EDGE = 0.03
POLL_INTERVAL = 30
MAX_TRADES_PER_HOUR = 5
SETTLEMENT_CHECK_INTERVAL = 60  # Check settlements every 60 seconds

POLYMARKET_API = "https://gamma-api.polymarket.com"

class RealSettlementTracker:
    def __init__(self):
        self.capital = INITIAL_CAPITAL
        self.available_capital = INITIAL_CAPITAL  # Capital not in open positions
        self.open_positions = []  # Positions waiting for settlement
        self.settled_trades = []  # Completed trades with real outcomes
        self.start_time = datetime.now(timezone.utc)
        self.last_trade_time = None
        self.hourly_trades = 0
        self.last_hour_reset = datetime.now(timezone.utc)
        self.traded_market_ids = set()
        
        os.makedirs('logs', exist_ok=True)
        self.state_file = 'logs/real_settlement_state.json'
        self.trades_file = 'logs/real_settlement_trades.jsonl'
        
        # Load existing state if available
        self.load_state()
        
        self.log("=" * 60)
        self.log("📊 REAL SETTLEMENT TRACKER")
        self.log("   NO FAKE WINS/LOSSES - Tracks real outcomes only")
        self.log(f"   Capital: ${self.capital:.2f}")
        self.log(f"   Open Positions: {len(self.open_positions)}")
        self.log("=" * 60)
    
    def log(self, msg: str):
        now = datetime.now(timezone.utc).strftime('%H:%M:%S')
        print(f"[{now}] {msg}", flush=True)
    
    def load_state(self):
        """Load existing state from file"""
        try:
            if os.path.exists(self.state_file):
                with open(self.state_file, 'r') as f:
                    state = json.load(f)
                    self.capital = state.get('capital', INITIAL_CAPITAL)
                    self.available_capital = state.get('available_capital', INITIAL_CAPITAL)
                    self.open_positions = state.get('open_positions', [])
                    self.settled_trades = state.get('settled_trades', [])
                    self.traded_market_ids = set(state.get('traded_market_ids', []))
                    self.log(f"📂 Loaded state: {len(self.open_positions)} open, {len(self.settled_trades)} settled")
        except Exception as e:
            self.log(f"⚠️ Could not load state: {e}")
    
    def save_state(self):
        """Save current state"""
        realized_pnl = sum(t.get('pnl', 0) for t in self.settled_trades)
        unrealized_value = sum(p.get('position_size', 0) for p in self.open_positions)
        
        state = {
            'mode': 'REAL_SETTLEMENT_TRACKER',
            'capital': round(self.capital, 2),
            'available_capital': round(self.available_capital, 2),
            'initial_capital': INITIAL_CAPITAL,
            'realized_pnl': round(realized_pnl, 2),
            'unrealized_positions': round(unrealized_value, 2),
            'open_positions_count': len(self.open_positions),
            'settled_trades_count': len(self.settled_trades),
            'wins': sum(1 for t in self.settled_trades if t.get('pnl', 0) > 0),
            'losses': sum(1 for t in self.settled_trades if t.get('pnl', 0) < 0),
            'start_time': self.start_time.isoformat(),
            'last_update': datetime.now(timezone.utc).isoformat(),
            'open_positions': self.open_positions,
            'recent_settled': self.settled_trades[-5:],
            'traded_market_ids': list(self.traded_market_ids)
        }
        
        with open(self.state_file, 'w') as f:
            json.dump(state, f, indent=2)
    
    def log_trade(self, trade: dict):
        """Append trade to log file"""
        with open(self.trades_file, 'a') as f:
            f.write(json.dumps(trade) + '\n')
    
    def get_markets(self) -> List[dict]:
        """Get active markets"""
        try:
            resp = requests.get(
                f"{POLYMARKET_API}/markets",
                params={"closed": "false", "limit": 100},
                timeout=15
            )
            resp.raise_for_status()
            return resp.json()
        except Exception as e:
            self.log(f"❌ API Error: {e}")
            return []
    
    def get_market_by_id(self, market_id: str) -> Optional[dict]:
        """Get specific market by ID"""
        try:
            resp = requests.get(
                f"{POLYMARKET_API}/markets/{market_id}",
                timeout=10
            )
            if resp.status_code == 200:
                return resp.json()
        except:
            pass
        return None
    
    def parse_prices(self, prices_raw) -> Optional[tuple]:
        """Parse outcome prices"""
        try:
            if isinstance(prices_raw, str):
                prices = json.loads(prices_raw)
            else:
                prices = prices_raw
            if len(prices) >= 2:
                return float(prices[0]), float(prices[1])
        except:
            pass
        return None
    
    def calculate_opportunity(self, market: dict) -> Optional[dict]:
        """Find trading opportunity"""
        volume = float(market.get('volume', 0) or 0)
        liquidity = float(market.get('liquidity', 0) or 0)
        
        if volume < 50000:
            return None
        
        prices = self.parse_prices(market.get('outcomePrices'))
        if not prices:
            return None
        
        yes_price, no_price = prices
        
        if yes_price <= 0.08 or yes_price >= 0.92:
            return None
        
        # Simple edge based on market inefficiency
        market_efficiency = min(liquidity / volume, 1.0) if volume > 0 else 0
        edge = 0.02 + (1 - market_efficiency) * 0.05
        edge = max(MIN_EDGE, min(edge, 0.10))
        
        if edge < MIN_EDGE:
            return None
        
        # Decide side
        if yes_price < 0.5:
            side = 'Yes'
            price = yes_price
        else:
            side = 'No'
            price = no_price
        
        return {
            'side': side,
            'price': price,
            'edge': edge,
            'volume': volume,
            'liquidity': liquidity
        }
    
    def find_opportunity(self, markets: List[dict]) -> Optional[dict]:
        """Find best opportunity"""
        opportunities = []
        
        for market in markets:
            market_id = market.get('id')
            if market_id in self.traded_market_ids:
                continue
            
            opp = self.calculate_opportunity(market)
            if opp:
                opportunities.append({
                    'market': market,
                    **opp
                })
        
        if not opportunities:
            return None
        
        return max(opportunities, key=lambda x: x['edge'] * (x['volume'] ** 0.2))
    
    def open_position(self, opp: dict) -> Optional[dict]:
        """Open a position (record entry, NO outcome simulation)"""
        now = datetime.now(timezone.utc)
        
        # Rate limiting
        if self.last_trade_time:
            elapsed = (now - self.last_trade_time).total_seconds()
            if elapsed < 60:
                return None
        
        if (now - self.last_hour_reset).total_seconds() > 3600:
            self.hourly_trades = 0
            self.last_hour_reset = now
        
        if self.hourly_trades >= MAX_TRADES_PER_HOUR:
            return None
        
        # Position sizing
        kelly = min(opp['edge'] * 2.5, 0.10)
        position_size = self.available_capital * kelly
        position_size = max(5.0, min(position_size, self.available_capital * 0.15))
        
        if position_size > self.available_capital:
            return None
        
        market = opp['market']
        shares = position_size / opp['price']
        
        position = {
            'id': f"POS_{now.strftime('%Y%m%d_%H%M%S')}",
            'opened_at': now.isoformat(),
            'market_id': market.get('id'),
            'question': market.get('question', ''),
            'side': opp['side'],
            'entry_price': round(opp['price'], 4),
            'position_size': round(position_size, 2),
            'shares': round(shares, 4),
            'edge': round(opp['edge'], 4),
            'status': 'OPEN',  # OPEN -> SETTLED
            'settlement_outcome': None,  # Will be filled when market settles
            'pnl': None  # Will be calculated on settlement
        }
        
        # Deduct from available capital
        self.available_capital -= position_size
        self.last_trade_time = now
        self.hourly_trades += 1
        self.traded_market_ids.add(market.get('id'))
        self.open_positions.append(position)
        
        self.log("=" * 50)
        self.log(f"📝 POSITION OPENED: {position['id']}")
        self.log(f"   {opp['side']} @ ${opp['price']:.4f}")
        self.log(f"   Size: ${position_size:.2f} ({shares:.2f} shares)")
        self.log(f"   Edge: {opp['edge']*100:.1f}%")
        self.log(f"   Market: {market.get('question', '')[:50]}...")
        self.log(f"   ⏳ WAITING FOR REAL SETTLEMENT")
        self.log(f"   Available: ${self.available_capital:.2f}")
        self.log("=" * 50)
        
        self.log_trade(position)
        self.save_state()
        return position
    
    def check_settlements(self):
        """Check if any open positions have settled"""
        if not self.open_positions:
            return
        
        self.log(f"🔍 Checking {len(self.open_positions)} open positions...")
        
        for i, pos in enumerate(self.open_positions[:]):  # Copy list for safe iteration
            market_id = pos.get('market_id')
            market = self.get_market_by_id(market_id)
            
            if not market:
                continue
            
            # Check if market is resolved
            resolved = market.get('resolved', False)
            resolved_outcome = market.get('resolvedOutcome')
            
            if resolved and resolved_outcome:
                # Market has settled!
                self.settle_position(pos, resolved_outcome)
    
    def settle_position(self, position: dict, resolved_outcome: str):
        """Settle a position with REAL outcome"""
        now = datetime.now(timezone.utc)
        
        # Determine if we won
        our_side = position.get('side')
        won = (our_side == resolved_outcome)
        
        # Calculate real PnL
        position_size = position.get('position_size', 0)
        shares = position.get('shares', 0)
        
        if won:
            # Win: shares pay out at $1
            payout = shares
            pnl = payout - position_size
        else:
            # Lose: shares worth $0
            payout = 0
            pnl = -position_size
        
        # Update capital
        self.capital += pnl
        self.available_capital += payout  # Return the payout (0 if lost)
        
        # Update position record
        position['status'] = 'SETTLED'
        position['settlement_outcome'] = resolved_outcome
        position['settled_at'] = now.isoformat()
        position['won'] = won
        position['pnl'] = round(pnl, 2)
        
        # Move to settled trades
        self.open_positions.remove(position)
        self.settled_trades.append(position)
        
        emoji = "✅" if won else "❌"
        self.log("")
        self.log("=" * 50)
        self.log(f"{emoji} POSITION SETTLED - {'WON' if won else 'LOST'}")
        self.log(f"   Market: {position.get('question', '')[:40]}...")
        self.log(f"   Our Side: {our_side} | Outcome: {resolved_outcome}")
        self.log(f"   PnL: ${pnl:+.2f}")
        self.log(f"   Capital: ${self.capital:.2f}")
        self.log("=" * 50)
        self.log("")
        
        self.log_trade(position)
        self.save_state()
    
    def print_status(self):
        """Print current status"""
        realized_pnl = sum(t.get('pnl', 0) for t in self.settled_trades)
        wins = sum(1 for t in self.settled_trades if t.get('pnl', 0) > 0)
        losses = sum(1 for t in self.settled_trades if t.get('pnl', 0) < 0)
        total = len(self.settled_trades)
        unrealized = sum(p.get('position_size', 0) for p in self.open_positions)
        
        self.log("")
        self.log("=" * 60)
        self.log("📊 STATUS REPORT")
        self.log("=" * 60)
        self.log(f"   Initial Capital: ${INITIAL_CAPITAL:.2f}")
        self.log(f"   Current Capital: ${self.capital:.2f}")
        self.log(f"   Available: ${self.available_capital:.2f}")
        self.log(f"   In Positions: ${unrealized:.2f}")
        self.log("")
        self.log(f"   Open Positions: {len(self.open_positions)}")
        self.log(f"   Settled Trades: {total}")
        if total > 0:
            self.log(f"   Realized PnL: ${realized_pnl:+.2f}")
            self.log(f"   Wins: {wins} | Losses: {losses} | Win Rate: {wins/total*100:.0f}%")
        self.log("=" * 60)
        self.log("")
    
    def run(self, duration_hours: float = 24.0):
        """Run tracker"""
        end_time = self.start_time + timedelta(hours=duration_hours)
        last_status = self.start_time
        last_settlement_check = self.start_time
        
        self.log(f"🏃 Running for {duration_hours} hours...")
        self.log("   Will track REAL settlements only - no fake outcomes")
        
        while datetime.now(timezone.utc) < end_time:
            try:
                now = datetime.now(timezone.utc)
                
                # Check settlements periodically
                if (now - last_settlement_check).total_seconds() >= SETTLEMENT_CHECK_INTERVAL:
                    self.check_settlements()
                    last_settlement_check = now
                
                # Look for new opportunities
                markets = self.get_markets()
                if markets:
                    opp = self.find_opportunity(markets)
                    if opp:
                        self.open_position(opp)
                
                # Status report every 10 minutes
                if (now - last_status).total_seconds() >= 600:
                    self.print_status()
                    last_status = now
                
                self.save_state()
                time.sleep(POLL_INTERVAL)
                
            except KeyboardInterrupt:
                self.log("⏹️ Stopped")
                break
            except Exception as e:
                self.log(f"❌ Error: {e}")
                time.sleep(POLL_INTERVAL)
        
        self.print_status()
        return self.settled_trades

if __name__ == "__main__":
    import argparse
    parser = argparse.ArgumentParser()
    parser.add_argument('--hours', type=float, default=24.0, help='Duration in hours')
    args = parser.parse_args()
    
    tracker = RealSettlementTracker()
    tracker.run(duration_hours=args.hours)
