use near_sdk::json_types::U64;
use near_sdk::NearToken;

use crate::math::mul_div;
use crate::test_utils::*;
use crate::{Markets, Status, NO, YES};

/// Alice bets 2 NEAR and Bob 1 NEAR on Yes, Carol 3 NEAR on No; returns the stored amounts.
fn market_with_bets() -> (Markets, U64, [NearToken; 3]) {
    let (mut markets, id) = open_market();
    let alice = bet(&mut markets, id, "alice.testnet", YES, near(2));
    let bob = bet(&mut markets, id, "bob.testnet", YES, near(1));
    let carol = bet(&mut markets, id, "carol.testnet", NO, near(3));
    (markets, id, [alice, bob, carol])
}

fn resolve(markets: &mut Markets, id: U64, outcome: u8) {
    call_as("paul.testnet", near(2), CLOSES_AT);
    markets.propose(id, outcome);
    call_as("anyone.testnet", zero(), CLOSES_AT + CHALLENGE_NS);
    markets.finalize_proposal(id);
}

fn claim(markets: &mut Markets, id: U64, name: &str) -> NearToken {
    call_as(name, zero(), CLOSES_AT + CHALLENGE_NS);
    markets.claim(id)
}

fn total(amounts: &[NearToken]) -> u128 {
    amounts.iter().map(|amount| amount.as_yoctonear()).sum()
}

#[test]
fn winners_share_the_whole_pool_in_proportion_to_their_bets() {
    let (mut markets, id, [alice, bob, carol]) = market_with_bets();
    resolve(&mut markets, id, YES);
    let pool = total(&[alice, bob, carol]);
    let yes = total(&[alice, bob]);

    let paid = claim(&mut markets, id, "alice.testnet");

    assert_eq!(paid.as_yoctonear(), mul_div(alice.as_yoctonear(), pool, yes));
    assert_eq!(transfers(), vec![("alice.testnet".to_string(), paid.as_yoctonear())]);
    let bobs = claim(&mut markets, id, "bob.testnet");
    assert_eq!(bobs.as_yoctonear(), mul_div(bob.as_yoctonear(), pool, yes));
    // Rounding down never pays out more than the pool.
    assert!(paid.as_yoctonear() + bobs.as_yoctonear() <= pool);
}

#[test]
fn a_losing_bet_claims_nothing_and_is_cleared() {
    let (mut markets, id, _) = market_with_bets();
    resolve(&mut markets, id, YES);

    assert_eq!(claim(&mut markets, id, "carol.testnet"), zero());
    assert!(transfers().is_empty());
    assert_eq!(markets.bet_of(id, account("carol.testnet")), None);
}

#[test]
fn no_ruling_refunds_every_bet() {
    let (mut markets, id, [alice, _, carol]) = market_with_bets();
    call_as("paul.testnet", near(2), CLOSES_AT);
    markets.propose(id, YES);
    call_as("chloe.testnet", near(5), CLOSES_AT);
    let _ = markets.challenge(id);
    call_as("market.testnet", zero(), CLOSES_AT);
    markets.on_dispute_created(id, Ok(U64(7)));
    call_as("court.testnet", zero(), CLOSES_AT);
    markets.on_ruling(U64(7), None);
    assert_eq!(markets.market(id).unwrap().status, Status::Resolved { outcome: None });

    assert_eq!(claim(&mut markets, id, "alice.testnet"), alice);
    assert_eq!(claim(&mut markets, id, "carol.testnet"), carol);
}

#[test]
fn bets_are_refunded_when_nobody_backed_the_outcome() {
    let (mut markets, id) = open_market();
    let alice = bet(&mut markets, id, "alice.testnet", YES, near(2));
    resolve(&mut markets, id, NO);

    assert_eq!(claim(&mut markets, id, "alice.testnet"), alice);
}

#[test]
fn claimable_shows_the_payout_before_claiming() {
    let (mut markets, id, [alice, bob, carol]) = market_with_bets();
    assert_eq!(markets.claimable(id, account("alice.testnet")), zero());
    resolve(&mut markets, id, YES);

    let shown = markets.claimable(id, account("alice.testnet"));

    let expected = mul_div(alice.as_yoctonear(), total(&[alice, bob, carol]), total(&[alice, bob]));
    assert_eq!(shown.as_yoctonear(), expected);
    assert_eq!(claim(&mut markets, id, "alice.testnet"), shown);
}

#[test]
#[should_panic(expected = "This market is not resolved yet")]
fn claim_refuses_an_unresolved_market() {
    let (mut markets, id, _) = market_with_bets();

    claim(&mut markets, id, "alice.testnet");
}

#[test]
#[should_panic(expected = "You have nothing to claim in this market")]
fn claim_refuses_a_second_claim() {
    let (mut markets, id, _) = market_with_bets();
    resolve(&mut markets, id, YES);
    claim(&mut markets, id, "alice.testnet");

    claim(&mut markets, id, "alice.testnet");
}
