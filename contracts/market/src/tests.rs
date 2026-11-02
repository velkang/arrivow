use near_sdk::json_types::U64;
use near_sdk::NearToken;

use super::*;
use crate::test_utils::*;

#[test]
fn create_market_opens_a_market_and_returns_what_storage_did_not_use() {
    let (markets, id) = open_market();

    let market = markets.market(id).unwrap();
    assert_eq!(id, U64(0));
    assert_eq!((market.creator, market.closes_at, market.status), (account("maya.testnet"), U64(CLOSES_AT), Status::Open));
    assert_eq!(market.pools, [zero(), zero()]);
    let refunds = transfers();
    assert_eq!(refunds.len(), 1);
    assert_eq!(refunds[0].0, "maya.testnet");
    assert!(refunds[0].1 > near(1).as_yoctonear() - NearToken::from_millinear(20).as_yoctonear());
}

#[test]
#[should_panic(expected = "Attach at least")]
fn create_market_refuses_a_deposit_below_its_storage() {
    let mut markets = markets();
    call_as("maya.testnet", NearToken::from_yoctonear(1), 0);

    markets.create_market("Q".into(), U64(CLOSES_AT));
}

#[test]
#[should_panic(expected = "A market must close in the future")]
fn create_market_refuses_a_closing_time_that_has_passed() {
    let mut markets = markets();
    call_as("maya.testnet", near(1), CLOSES_AT);

    markets.create_market("Q".into(), U64(CLOSES_AT));
}

#[test]
fn bet_adds_the_deposit_less_first_bet_storage_to_the_pool() {
    let (mut markets, id) = open_market();

    let first = bet(&mut markets, id, "alice.testnet", YES, near(2));
    let second = bet(&mut markets, id, "alice.testnet", YES, near(1));

    // Only the first bet pays for the bet's storage.
    assert!(first < near(2) && first > near(2).saturating_sub(NearToken::from_millinear(10)));
    assert_eq!(second, first.saturating_add(near(1)));
    assert_eq!(markets.market(id).unwrap().pools, [second, zero()]);
    assert_eq!(markets.bet_of(id, account("alice.testnet")).unwrap().amounts, [second, zero()]);
}

#[test]
fn bets_on_both_sides_are_kept_apart() {
    let (mut markets, id) = open_market();

    let yes = bet(&mut markets, id, "alice.testnet", YES, near(2));
    let no = bet(&mut markets, id, "bob.testnet", NO, near(3));

    assert_eq!(markets.market(id).unwrap().pools, [yes, no]);
}

#[test]
#[should_panic(expected = "Betting closed at")]
fn bet_refuses_once_the_market_closes() {
    let (mut markets, id) = open_market();
    call_as("alice.testnet", near(1), CLOSES_AT);

    markets.bet(id, YES);
}

#[test]
#[should_panic(expected = "Bet on 0 (Yes) or 1 (No)")]
fn bet_refuses_an_unknown_outcome() {
    let (mut markets, id) = open_market();
    call_as("alice.testnet", near(1), 0);

    markets.bet(id, 2);
}

#[test]
#[should_panic(expected = "Bet more than")]
fn bet_refuses_a_deposit_that_only_covers_storage() {
    let (mut markets, id) = open_market();
    call_as("alice.testnet", NearToken::from_yoctonear(1), 0);

    markets.bet(id, YES);
}

#[test]
fn markets_lists_a_page_of_markets() {
    let (mut markets, _) = open_market();
    call_as("maya.testnet", near(1), 0);
    markets.create_market("Second".into(), U64(CLOSES_AT));

    let page: Vec<U64> = markets.markets(Some(1), Some(5)).into_iter().map(|(id, _)| id).collect();

    assert_eq!(page, vec![U64(1)]);
}

#[test]
#[should_panic(expected = "proposal_bond must be more than 0")]
fn new_refuses_a_zero_bond() {
    call_as("market.testnet", zero(), 0);
    Markets::new(Config { proposal_bond: zero(), ..config() });
}
