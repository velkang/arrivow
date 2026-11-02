//! Helpers shared by the unit tests.

use near_sdk::json_types::{Base64VecU8, U64};
use near_sdk::test_utils::VMContextBuilder;
use near_sdk::{testing_env, AccountId, NearToken};

use crate::{commitment_hash, Config, Court};

pub const DELAY_NS: u64 = 60_000_000_000;
pub const COMMIT_NS: u64 = 120_000_000_000;
pub const REVEAL_NS: u64 = 120_000_000_000;
pub const APPEAL_NS: u64 = 60_000_000_000;
/// When the first round's voting and revealing close, for a dispute opened at 0.
pub const VOTING_CLOSED: u64 = COMMIT_NS;
pub const REVEAL_CLOSED: u64 = COMMIT_NS + REVEAL_NS;
pub const YES: u8 = 0;
pub const NO: u8 = 1;

pub fn near(amount: u128) -> NearToken {
    NearToken::from_near(amount)
}

pub fn zero() -> NearToken {
    NearToken::from_yoctonear(0)
}

pub fn account(name: &str) -> AccountId {
    name.parse().unwrap()
}

pub fn call_with_seed(name: &str, deposit: NearToken, now_ns: u64, seed: u8) {
    testing_env!(VMContextBuilder::new()
        .current_account_id(account("court.testnet"))
        .predecessor_account_id(account(name))
        .attached_deposit(deposit)
        .block_timestamp(now_ns)
        .random_seed([seed; 32])
        .build());
}

pub fn call_as(name: &str, deposit: NearToken, now_ns: u64) {
    call_with_seed(name, deposit, now_ns, 0);
}

pub fn config() -> Config {
    Config {
        min_stake: near(1),
        unstake_delay_ns: U64(DELAY_NS),
        juror_lock: near(1),
        first_round_jurors: 3,
        max_jury_size: 7,
        min_fee: near(1),
        appeal_fee_per_juror: NearToken::from_millinear(500),
        commit_period_ns: U64(COMMIT_NS),
        reveal_period_ns: U64(REVEAL_NS),
        appeal_period_ns: U64(APPEAL_NS),
    }
}

pub fn court() -> Court {
    call_as("court.testnet", zero(), 0);
    Court::new(config())
}

/// A court where each named account has staked the given amount.
pub fn court_with_jurors(jurors: &[(&str, NearToken)]) -> Court {
    let mut court = court();
    for (name, amount) in jurors {
        call_as(name, *amount, 0);
        court.stake();
    }
    court
}

pub fn outcomes() -> Vec<String> {
    vec!["Yes".into(), "No".into()]
}

/// Alice, Bob and Carol with 5 NEAR each, and one dispute with a 3 NEAR deposit that draws all three.
pub fn three_juror_dispute() -> (Court, U64) {
    let mut court = court_with_jurors(&[("alice.testnet", near(5)), ("bob.testnet", near(5)), ("carol.testnet", near(5))]);
    call_as("market.testnet", near(3), 0);
    let id = court.create_dispute("Did it rain in Lagos on Nov 1?".into(), outcomes());
    (court, id)
}

pub fn salt(name: &str) -> Vec<u8> {
    format!("salt of {name}").into_bytes()
}

fn last_round_times(court: &Court, id: U64) -> (u64, u64) {
    let round = court.dispute(id).unwrap().rounds.last().cloned().unwrap();
    (round.commit_ends.0, round.reveal_ends.0)
}

/// Commits `name`'s vote in the dispute's current round, while its voting is open.
pub fn commit(court: &mut Court, id: U64, name: &str, outcome: u8) {
    let (commit_ends, _) = last_round_times(court, id);
    call_as(name, zero(), commit_ends - COMMIT_NS);
    court.commit(id, Base64VecU8(commitment_hash(outcome, &salt(name), &account(name))));
}

/// Reveals `name`'s vote in the current round, as soon as voting closes.
pub fn reveal(court: &mut Court, id: U64, name: &str, outcome: u8) {
    let (commit_ends, _) = last_round_times(court, id);
    call_as(name, zero(), commit_ends);
    court.reveal(id, outcome, Base64VecU8(salt(name)));
}

pub fn vote(court: &mut Court, id: U64, name: &str, outcome: u8) {
    commit(court, id, name, outcome);
    reveal(court, id, name, outcome);
}

/// Tallies the current round as soon as revealing closes.
pub fn tally(court: &mut Court, id: U64) -> crate::Dispute {
    let (_, reveal_ends) = last_round_times(court, id);
    call_as("anyone.testnet", zero(), reveal_ends);
    court.tally(id)
}

pub fn staked(court: &Court, name: &str) -> NearToken {
    court.juror(account(name)).unwrap().staked
}

pub fn locked(court: &Court, name: &str) -> NearToken {
    court.juror(account(name)).unwrap().locked
}

pub fn total_staked(court: &Court, names: &[&str]) -> u128 {
    names.iter().map(|name| staked(court, name).as_yoctonear()).sum()
}
