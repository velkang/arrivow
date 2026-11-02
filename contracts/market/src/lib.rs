//! Pooled Yes/No prediction markets whose disputed outcomes are settled by the court.

use near_sdk::json_types::U64;
use near_sdk::store::{LookupMap, Vector};
use near_sdk::{env, near, require, AccountId, BorshStorageKey, NearToken, PanicOnDefault, Promise};

mod claim;
mod math;
mod resolution;
#[cfg(test)]
mod test_utils;

pub use resolution::Court;

pub const YES: u8 = 0;
pub const NO: u8 = 1;
const OUTCOMES: usize = 2;
const MAX_QUESTION_BYTES: usize = 500;

#[near(serializers = [borsh])]
#[derive(BorshStorageKey)]
enum StorageKey {
    Markets,
    Bets,
    DisputedMarkets,
}

#[near(serializers = [json, borsh])]
#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    /// The court contract that settles challenged outcomes.
    pub court: AccountId,
    /// What proposing an outcome, and challenging one, each put at stake.
    pub proposal_bond: NearToken,
    /// Sent to the court with each challenge; it must cover the court's fee and storage.
    pub court_fee: NearToken,
    /// How long a proposed outcome can be challenged, in nanoseconds.
    pub challenge_period_ns: U64,
}

#[near(serializers = [json, borsh])]
#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    /// Taking bets until `closes_at`, then waiting for a proposed outcome.
    Open,
    Proposed {
        outcome: u8,
        proposer: AccountId,
        challenge_ends: U64,
    },
    /// A challenge is waiting for the court to open its dispute.
    Challenging {
        outcome: u8,
        proposer: AccountId,
        challenge_ends: U64,
        challenger: AccountId,
    },
    Disputed {
        outcome: u8,
        proposer: AccountId,
        challenger: AccountId,
        dispute_id: U64,
    },
    /// `outcome` is None when the court gave no ruling; every bet is then refunded.
    Resolved { outcome: Option<u8> },
}

#[near(serializers = [json, borsh])]
#[derive(Clone, Debug, PartialEq)]
pub struct Market {
    pub creator: AccountId,
    pub question: String,
    pub closes_at: U64,
    /// Total staked on Yes and on No.
    pub pools: [NearToken; OUTCOMES],
    pub status: Status,
}

#[near(serializers = [json, borsh])]
#[derive(Clone, Debug, PartialEq)]
pub struct Bet {
    /// What this account staked on Yes and on No.
    pub amounts: [NearToken; OUTCOMES],
}

#[near(contract_state)]
#[derive(PanicOnDefault)]
pub struct Markets {
    config: Config,
    markets: Vector<Market>,
    bets: LookupMap<(u64, AccountId), Bet>,
    /// Court dispute id → market id, so `on_ruling` can find its market.
    disputed: LookupMap<u64, u64>,
}

#[near]
impl Markets {
    #[init]
    pub fn new(config: Config) -> Self {
        require!(!config.proposal_bond.is_zero(), "proposal_bond must be more than 0");
        require!(!config.court_fee.is_zero(), "court_fee must be more than 0");
        require!(config.challenge_period_ns.0 > 0, "challenge_period_ns must be more than 0");
        Self {
            config,
            markets: Vector::new(StorageKey::Markets),
            bets: LookupMap::new(StorageKey::Bets),
            disputed: LookupMap::new(StorageKey::DisputedMarkets),
        }
    }

    /// Opens a market that takes bets until `closes_at`. The deposit pays for its storage;
    /// anything above that is returned.
    #[payable]
    pub fn create_market(&mut self, question: String, closes_at: U64) -> U64 {
        require!(
            !question.is_empty() && question.len() <= MAX_QUESTION_BYTES,
            format!("A question is 1 to {MAX_QUESTION_BYTES} bytes")
        );
        require!(closes_at.0 > env::block_timestamp(), "A market must close in the future");
        let creator = env::predecessor_account_id();
        let market = Market {
            creator: creator.clone(),
            question,
            closes_at,
            pools: [NearToken::from_yoctonear(0); OUTCOMES],
            status: Status::Open,
        };
        let id = self.markets.len() as u64;
        let storage = storage_cost_of(|| {
            self.markets.push(market);
            self.markets.flush();
        });
        let deposit = env::attached_deposit();
        require!(deposit >= storage, format!("Attach at least {storage} for the market's storage"));
        pay(&creator, deposit.saturating_sub(storage));
        U64(id)
    }

    /// Stakes the deposit on `outcome`. A first bet in a market pays for its storage out of the deposit.
    #[payable]
    pub fn bet(&mut self, market_id: U64, outcome: u8) -> Bet {
        require!((outcome as usize) < OUTCOMES, "Bet on 0 (Yes) or 1 (No)");
        let market = self.market_or_panic(market_id);
        require!(
            market.status == Status::Open && env::block_timestamp() < market.closes_at.0,
            format!("Betting closed at {} ns", market.closes_at.0)
        );
        let key = (market_id.0, env::predecessor_account_id());
        let current = self.bets.get(&key).cloned().unwrap_or(Bet {
            amounts: [NearToken::from_yoctonear(0); OUTCOMES],
        });
        let storage = storage_cost_of(|| {
            self.bets.insert(key.clone(), current.clone());
            self.bets.flush();
        });
        let deposit = env::attached_deposit();
        require!(deposit > storage, format!("Bet more than the {storage} its storage costs"));
        let amount = deposit.saturating_sub(storage);
        let bet = Bet {
            amounts: added(current.amounts, outcome, amount),
        };
        self.bets.insert(key, bet.clone());
        self.save_market(
            market_id,
            Market {
                pools: added(market.pools, outcome, amount),
                ..market
            },
        );
        bet
    }

    pub fn config(&self) -> Config {
        self.config.clone()
    }

    pub fn market(&self, market_id: U64) -> Option<Market> {
        self.markets.get(market_id.0 as u32).cloned()
    }

    pub fn markets(&self, from_index: Option<u32>, limit: Option<u32>) -> Vec<(U64, Market)> {
        self.markets
            .iter()
            .enumerate()
            .skip(from_index.unwrap_or(0) as usize)
            .take(limit.unwrap_or(50).min(100) as usize)
            .map(|(index, market)| (U64(index as u64), market.clone()))
            .collect()
    }

    pub fn bet_of(&self, market_id: U64, account_id: AccountId) -> Option<Bet> {
        self.bets.get(&(market_id.0, account_id)).cloned()
    }
}

impl Markets {
    fn market_or_panic(&self, market_id: U64) -> Market {
        self.market(market_id).unwrap_or_else(|| env::panic_str("There is no such market"))
    }

    fn save_market(&mut self, market_id: U64, market: Market) {
        self.markets.replace(market_id.0 as u32, market);
    }
}

/// What the storage written by `write` costs, at the network's price per byte.
fn storage_cost_of(write: impl FnOnce()) -> NearToken {
    let before = env::storage_usage();
    write();
    let added = env::storage_usage().saturating_sub(before);
    env::storage_byte_cost().saturating_mul(added as u128)
}

/// `amounts` with `amount` added to `outcome`.
fn added(amounts: [NearToken; OUTCOMES], outcome: u8, amount: NearToken) -> [NearToken; OUTCOMES] {
    std::array::from_fn(|index| {
        if index == outcome as usize {
            amounts[index].saturating_add(amount)
        } else {
            amounts[index]
        }
    })
}

fn pay(account_id: &AccountId, amount: NearToken) {
    if !amount.is_zero() {
        Promise::new(account_id.clone()).transfer(amount).detach();
    }
}

#[cfg(test)]
mod tests;
