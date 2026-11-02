//! A court that settles disputes with juries drawn at random from staked jurors.

use near_sdk::json_types::U64;
use near_sdk::store::{IterableMap, LookupMap};
use near_sdk::{near, require, AccountId, BorshStorageKey, NearToken, PanicOnDefault};

mod dispute;
mod ruling;
mod staking;
#[cfg(test)]
mod test_utils;

pub use dispute::{commitment_hash, Appeal, Dispute, Round};
pub use ruling::Arbitrable;
pub use staking::{Juror, Unstaking};

/// Most jurors the court holds, so a draw that reads every juror stays within gas.
pub const MAX_JURORS: u32 = 200;

#[near(serializers = [borsh])]
#[derive(BorshStorageKey)]
enum StorageKey {
    Jurors,
    Disputes,
}

#[near(serializers = [json, borsh])]
#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    /// Smallest stake a juror may hold.
    pub min_stake: NearToken,
    /// How long stake asked back waits before it can be withdrawn, in nanoseconds.
    pub unstake_delay_ns: U64,
    /// Stake locked from each drawn juror until their round is settled.
    pub juror_lock: NearToken,
    /// Jurors drawn for a dispute's first round; each appeal draws twice as many plus one.
    pub first_round_jurors: u32,
    /// The largest jury an appeal can draw; that round's outcome is final.
    pub max_jury_size: u32,
    /// Smallest fee a dispute must attach; it pays the jurors who vote with the final ruling.
    pub min_fee: NearToken,
    /// An appeal pays this for each juror of the larger jury.
    pub appeal_fee_per_juror: NearToken,
    /// How long drawn jurors have to commit their vote, in nanoseconds.
    pub commit_period_ns: U64,
    /// How long they then have to reveal it, in nanoseconds.
    pub reveal_period_ns: U64,
    /// How long a counted round's outcome can be appealed, in nanoseconds.
    pub appeal_period_ns: U64,
}

#[near(contract_state)]
#[derive(PanicOnDefault)]
pub struct Court {
    config: Config,
    jurors: IterableMap<AccountId, Juror>,
    disputes: LookupMap<u64, Dispute>,
    next_dispute_id: u64,
}

#[near]
impl Court {
    #[init]
    pub fn new(config: Config) -> Self {
        require!(!config.juror_lock.is_zero(), "juror_lock must be more than 0");
        require!(config.min_stake >= config.juror_lock, "min_stake must be at least juror_lock");
        require!(config.first_round_jurors % 2 == 1, "first_round_jurors must be odd");
        require!(
            config.max_jury_size % 2 == 1 && config.max_jury_size >= config.first_round_jurors,
            "max_jury_size must be odd and at least first_round_jurors"
        );
        require!(config.max_jury_size <= MAX_JURORS, "max_jury_size must not exceed MAX_JURORS");
        require!(
            config.commit_period_ns.0 > 0 && config.reveal_period_ns.0 > 0,
            "Voting and revealing must each last more than 0 ns"
        );
        Self {
            config,
            jurors: IterableMap::new(StorageKey::Jurors),
            disputes: LookupMap::new(StorageKey::Disputes),
            next_dispute_id: 0,
        }
    }

    pub fn config(&self) -> Config {
        self.config.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::{call_as, config, zero};

    #[test]
    #[should_panic(expected = "juror_lock must be more than 0")]
    fn new_refuses_a_zero_lock() {
        call_as("court.testnet", zero(), 0);
        Court::new(Config { juror_lock: zero(), ..config() });
    }

    #[test]
    #[should_panic(expected = "first_round_jurors must be odd")]
    fn new_refuses_an_even_jury() {
        call_as("court.testnet", zero(), 0);
        Court::new(Config { first_round_jurors: 4, ..config() });
    }

    #[test]
    #[should_panic(expected = "Voting and revealing must each last more than 0 ns")]
    fn new_refuses_zero_length_rounds() {
        call_as("court.testnet", zero(), 0);
        Court::new(Config { commit_period_ns: U64(0), ..config() });
    }

    #[test]
    #[should_panic(expected = "min_stake must be at least juror_lock")]
    fn new_refuses_a_minimum_stake_below_the_lock() {
        call_as("court.testnet", zero(), 0);
        Court::new(Config { min_stake: NearToken::from_millinear(500), ..config() });
    }
}
