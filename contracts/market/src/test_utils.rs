//! Helpers shared by the unit tests.

use near_sdk::json_types::U64;
use near_sdk::mock::MockAction;
use near_sdk::test_utils::{get_created_receipts, VMContextBuilder};
use near_sdk::{testing_env, AccountId, Gas, NearToken};

use crate::{Config, Markets};

pub const CLOSES_AT: u64 = 1_000;
pub const CHALLENGE_NS: u64 = 60_000_000_000;

pub fn near(amount: u128) -> NearToken {
    NearToken::from_near(amount)
}

pub fn zero() -> NearToken {
    NearToken::from_yoctonear(0)
}

pub fn account(name: &str) -> AccountId {
    name.parse().unwrap()
}

pub fn call_as(name: &str, deposit: NearToken, now_ns: u64) {
    testing_env!(VMContextBuilder::new()
        .current_account_id(account("market.testnet"))
        .predecessor_account_id(account(name))
        .attached_deposit(deposit)
        .block_timestamp(now_ns)
        .prepaid_gas(Gas::from_tgas(300))
        .build());
}

pub fn config() -> Config {
    Config {
        court: account("court.testnet"),
        proposal_bond: near(2),
        court_fee: near(3),
        challenge_period_ns: U64(CHALLENGE_NS),
    }
}

pub fn markets() -> Markets {
    call_as("market.testnet", zero(), 0);
    Markets::new(config())
}

/// A market that closes at `CLOSES_AT`, created by Maya.
pub fn open_market() -> (Markets, U64) {
    let mut markets = markets();
    call_as("maya.testnet", near(1), 0);
    let id = markets.create_market("Will it rain in Lagos on Nov 1?".into(), U64(CLOSES_AT));
    (markets, id)
}

pub fn bet(markets: &mut Markets, id: U64, name: &str, outcome: u8, amount: NearToken) -> NearToken {
    call_as(name, amount, 0);
    let bet = markets.bet(id, outcome);
    bet.amounts[outcome as usize]
}

/// Every transfer created by the last call, as (receiver, yoctoNEAR).
pub fn transfers() -> Vec<(String, u128)> {
    get_created_receipts()
        .into_iter()
        .flat_map(|receipt| {
            let to = receipt.receiver_id.to_string();
            receipt.actions.into_iter().filter_map(move |action| match action {
                MockAction::Transfer { deposit, .. } => Some((to.clone(), deposit.as_yoctonear())),
                _ => None,
            })
        })
        .collect()
}

/// Whether a receipt created by the last call runs `method` on `receiver`, and with what deposit.
pub fn call_deposit(receiver: &str, method: &str) -> Option<u128> {
    get_created_receipts().into_iter().find_map(|receipt| {
        if receipt.receiver_id.as_str() != receiver {
            return None;
        }
        receipt.actions.into_iter().find_map(|action| match action {
            MockAction::FunctionCallWeight { method_name, attached_deposit, .. } if method_name == method.as_bytes() => {
                Some(attached_deposit.as_yoctonear())
            }
            _ => None,
        })
    })
}
