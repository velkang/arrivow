use near_sdk::json_types::U64;
use near_sdk::{env, near, require, AccountId, NearToken};

use crate::math::mul_div;
use crate::{pay, Bet, Market, Markets, MarketsExt, Status};

#[cfg(test)]
mod tests;

#[near]
impl Markets {
    /// Pays out the caller's bet in a resolved market: a share of the whole pool for the winning
    /// side, or every bet back when there is no ruling or nobody backed the outcome.
    pub fn claim(&mut self, market_id: U64) -> NearToken {
        let account_id = env::predecessor_account_id();
        let market = self.market_or_panic(market_id);
        require!(matches!(market.status, Status::Resolved { .. }), "This market is not resolved yet");
        let key = (market_id.0, account_id.clone());
        let bet = self
            .bets
            .get(&key)
            .cloned()
            .unwrap_or_else(|| env::panic_str("You have nothing to claim in this market"));
        let payout = payout(&market, &bet);
        self.bets.remove(&key);
        pay(&account_id, payout);
        payout
    }

    /// What `claim` would pay `account_id` now; 0 while the market is unresolved.
    pub fn claimable(&self, market_id: U64, account_id: AccountId) -> NearToken {
        match (self.market(market_id), self.bet_of(market_id, account_id)) {
            (Some(market), Some(bet)) => payout(&market, &bet),
            _ => NearToken::from_yoctonear(0),
        }
    }
}

/// Rounded down, so the payouts never add up to more than the pool; the dust stays in the contract.
fn payout(market: &Market, bet: &Bet) -> NearToken {
    let Status::Resolved { outcome } = market.status else {
        return NearToken::from_yoctonear(0);
    };
    let total: u128 = market.pools.iter().map(|pool| pool.as_yoctonear()).sum();
    let amount = match outcome.map(usize::from) {
        Some(won) if !market.pools[won].is_zero() => {
            mul_div(bet.amounts[won].as_yoctonear(), total, market.pools[won].as_yoctonear())
        }
        _ => bet.amounts.iter().map(|amount| amount.as_yoctonear()).sum(),
    };
    NearToken::from_yoctonear(amount)
}
