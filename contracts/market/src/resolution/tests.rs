use near_sdk::json_types::U64;
use near_sdk::PromiseError;

use crate::test_utils::*;
use crate::{Markets, Status, NO, YES};

fn proposed(outcome: u8) -> (Markets, U64) {
    let (mut markets, id) = open_market();
    call_as("paul.testnet", near(2), CLOSES_AT);
    markets.propose(id, outcome);
    (markets, id)
}

fn challenged() -> (Markets, U64) {
    let (mut markets, id) = proposed(YES);
    call_as("chloe.testnet", near(5), CLOSES_AT);
    let _ = markets.challenge(id);
    (markets, id)
}

/// A YES proposal challenged into court dispute 7.
fn disputed() -> (Markets, U64) {
    let (mut markets, id) = challenged();
    call_as("market.testnet", zero(), CLOSES_AT);
    markets.on_dispute_created(id, Ok(U64(7)));
    (markets, id)
}

fn rule(markets: &mut Markets, ruling: Option<u8>) {
    call_as("court.testnet", zero(), CLOSES_AT);
    markets.on_ruling(U64(7), ruling);
}

#[test]
fn propose_opens_the_challenge_window() {
    let (markets, id) = proposed(YES);

    assert_eq!(
        markets.market(id).unwrap().status,
        Status::Proposed {
            outcome: YES,
            proposer: account("paul.testnet"),
            challenge_ends: U64(CLOSES_AT + CHALLENGE_NS),
        }
    );
}

#[test]
#[should_panic(expected = "Outcomes can be proposed once betting closes")]
fn propose_refuses_while_betting_is_open() {
    let (mut markets, id) = open_market();
    call_as("paul.testnet", near(2), CLOSES_AT - 1);

    markets.propose(id, YES);
}

#[test]
#[should_panic(expected = "Attach exactly the 2.00 NEAR bond")]
fn propose_refuses_a_different_bond() {
    let (mut markets, id) = open_market();
    call_as("paul.testnet", near(1), CLOSES_AT);

    markets.propose(id, YES);
}

#[test]
#[should_panic(expected = "This market is not waiting for a proposal")]
fn propose_refuses_a_second_proposal() {
    let (mut markets, id) = proposed(YES);
    call_as("pia.testnet", near(2), CLOSES_AT);

    markets.propose(id, NO);
}

#[test]
fn finalize_proposal_resolves_an_unchallenged_market_and_returns_the_bond() {
    let (mut markets, id) = proposed(NO);
    call_as("anyone.testnet", zero(), CLOSES_AT + CHALLENGE_NS);

    let market = markets.finalize_proposal(id);

    assert_eq!(market.status, Status::Resolved { outcome: Some(NO) });
    assert_eq!(transfers(), vec![("paul.testnet".to_string(), near(2).as_yoctonear())]);
}

#[test]
#[should_panic(expected = "Challenges close at")]
fn finalize_proposal_refuses_while_it_can_be_challenged() {
    let (mut markets, id) = proposed(YES);
    call_as("anyone.testnet", zero(), CLOSES_AT + CHALLENGE_NS - 1);

    markets.finalize_proposal(id);
}

#[test]
fn challenge_asks_the_court_for_a_dispute_with_the_court_fee() {
    let (markets, id) = challenged();

    assert_eq!(call_deposit("court.testnet", "create_dispute"), Some(near(3).as_yoctonear()));
    assert!(matches!(markets.market(id).unwrap().status, Status::Challenging { .. }));
}

#[test]
#[should_panic(expected = "Attach the 2.00 NEAR bond plus the 3.00 NEAR court fee")]
fn challenge_refuses_a_deposit_without_the_court_fee() {
    let (mut markets, id) = proposed(YES);
    call_as("chloe.testnet", near(2), CLOSES_AT);

    let _ = markets.challenge(id);
}

#[test]
#[should_panic(expected = "Challenges closed at")]
fn challenge_refuses_once_the_window_has_closed() {
    let (mut markets, id) = proposed(YES);
    call_as("chloe.testnet", near(5), CLOSES_AT + CHALLENGE_NS);

    let _ = markets.challenge(id);
}

#[test]
#[should_panic(expected = "This market has no proposal to challenge")]
fn challenge_refuses_a_market_already_being_challenged() {
    let (mut markets, id) = challenged();
    call_as("carl.testnet", near(5), CLOSES_AT);

    let _ = markets.challenge(id);
}

#[test]
fn on_dispute_created_records_the_courts_dispute() {
    let (markets, id) = disputed();

    assert_eq!(
        markets.market(id).unwrap().status,
        Status::Disputed {
            outcome: YES,
            proposer: account("paul.testnet"),
            challenger: account("chloe.testnet"),
            dispute_id: U64(7),
        }
    );
}

#[test]
fn a_failed_dispute_restores_the_proposal_and_refunds_the_challenger() {
    let (mut markets, id) = challenged();
    call_as("market.testnet", zero(), CLOSES_AT);

    assert!(!markets.on_dispute_created(id, Err(PromiseError::Failed)));

    assert!(matches!(markets.market(id).unwrap().status, Status::Proposed { outcome: YES, .. }));
    assert_eq!(transfers(), vec![("chloe.testnet".to_string(), near(5).as_yoctonear())]);
}

#[test]
fn a_ruling_for_the_proposal_gives_the_proposer_both_bonds() {
    let (mut markets, id) = disputed();

    rule(&mut markets, Some(YES));

    assert_eq!(markets.market(id).unwrap().status, Status::Resolved { outcome: Some(YES) });
    assert_eq!(transfers(), vec![("paul.testnet".to_string(), near(4).as_yoctonear())]);
}

#[test]
fn a_ruling_against_the_proposal_gives_the_challenger_both_bonds() {
    let (mut markets, id) = disputed();

    rule(&mut markets, Some(NO));

    assert_eq!(markets.market(id).unwrap().status, Status::Resolved { outcome: Some(NO) });
    assert_eq!(transfers(), vec![("chloe.testnet".to_string(), near(4).as_yoctonear())]);
}

#[test]
fn no_ruling_returns_each_bond_to_its_owner() {
    let (mut markets, id) = disputed();

    rule(&mut markets, None);

    assert_eq!(markets.market(id).unwrap().status, Status::Resolved { outcome: None });
    let mut paid = transfers();
    paid.sort();
    assert_eq!(
        paid,
        vec![("chloe.testnet".to_string(), near(2).as_yoctonear()), ("paul.testnet".to_string(), near(2).as_yoctonear())]
    );
}

#[test]
fn a_repeated_ruling_is_ignored() {
    let (mut markets, id) = disputed();
    rule(&mut markets, Some(YES));

    rule(&mut markets, Some(NO));

    assert_eq!(markets.market(id).unwrap().status, Status::Resolved { outcome: Some(YES) });
    assert!(transfers().is_empty());
}

#[test]
#[should_panic(expected = "Only the court can rule")]
fn on_ruling_refuses_anyone_but_the_court() {
    let (mut markets, _) = disputed();
    call_as("mallory.testnet", zero(), CLOSES_AT);

    markets.on_ruling(U64(7), Some(NO));
}

#[test]
#[should_panic(expected = "Attach at least 150 Tgas")]
fn challenge_refuses_too_little_gas_for_the_court() {
    let (mut markets, id) = proposed(YES);
    near_sdk::testing_env!(near_sdk::test_utils::VMContextBuilder::new()
        .current_account_id(account("market.testnet"))
        .predecessor_account_id(account("chloe.testnet"))
        .attached_deposit(near(5))
        .block_timestamp(CLOSES_AT)
        .prepaid_gas(near_sdk::Gas::from_tgas(50))
        .build());

    let _ = markets.challenge(id);
}

