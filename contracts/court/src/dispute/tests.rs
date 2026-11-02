use std::collections::HashSet;

use near_sdk::json_types::{Base64VecU8, U64};
use near_sdk::NearToken;

use super::*;
use crate::test_utils::*;

#[test]
fn create_dispute_draws_distinct_jurors_and_locks_their_stake() {
    let names = ["alice.testnet", "bob.testnet", "carol.testnet", "dave.testnet", "erin.testnet"];
    let mut court = court_with_jurors(&names.map(|name| (name, near(5))));
    call_with_seed("market.testnet", near(3), 10, 7);

    let id = court.create_dispute("Did it rain in Lagos on Nov 1?".into(), outcomes());

    let dispute = court.dispute(id).unwrap();
    let round = &dispute.rounds[0];
    assert_eq!(id, U64(0));
    assert_eq!((&dispute.arbitrable, dispute.finalized), (&account("market.testnet"), false));
    // The dispute's storage is paid out of the deposit, so slightly less than 3 NEAR is left as fee.
    assert!(round.fee < near(3) && round.fee > near(3).saturating_sub(NearToken::from_millinear(10)));
    assert_eq!(round.jurors.iter().collect::<HashSet<_>>().len(), 3);
    assert_eq!((round.commit_ends, round.reveal_ends), (U64(10 + COMMIT_NS), U64(10 + COMMIT_NS + REVEAL_NS)));
    for name in names {
        let drawn = round.jurors.contains(&account(name));
        assert_eq!(locked(&court, name), if drawn { near(1) } else { zero() }, "{name}");
    }
}

#[test]
fn create_dispute_numbers_disputes_in_order() {
    let names = ["a.testnet", "b.testnet", "c.testnet", "d.testnet", "e.testnet", "f.testnet"];
    let mut court = court_with_jurors(&names.map(|name| (name, near(5))));
    call_as("market.testnet", near(2), 0);

    let ids = [court.create_dispute("First".into(), outcomes()), court.create_dispute("Second".into(), outcomes())];

    assert_eq!(ids, [U64(0), U64(1)]);
}

#[test]
fn create_dispute_draws_by_stake() {
    for seed in 0..20 {
        // Each seed on its own thread, since the mocked chain's storage is per thread.
        let whale_drawn = std::thread::spawn(move || {
            let mut court = court_with_jurors(&[
                ("whale.testnet", near(1_000_000)),
                ("a.testnet", near(1)),
                ("b.testnet", near(1)),
                ("c.testnet", near(1)),
                ("d.testnet", near(1)),
            ]);
            call_with_seed("market.testnet", near(2), 0, seed);
            let id = court.create_dispute("Q".into(), outcomes());
            court.dispute(id).unwrap().rounds[0].jurors.contains(&account("whale.testnet"))
        })
        .join()
        .unwrap();

        assert!(whale_drawn, "seed {seed}");
    }
}

#[test]
#[should_panic(expected = "Only 0 jurors can be drawn; 3 are needed")]
fn create_dispute_skips_stake_that_is_already_locked() {
    let half = NearToken::from_millinear(1_500);
    let mut court = court_with_jurors(&[("alice.testnet", half), ("bob.testnet", half), ("carol.testnet", half)]);
    call_as("market.testnet", near(2), 0);
    court.create_dispute("First".into(), outcomes());

    court.create_dispute("Second".into(), outcomes());
}

#[test]
#[should_panic(expected = "Attach a fee of at least 1.00 NEAR plus")]
fn create_dispute_refuses_a_fee_below_the_minimum() {
    let mut court = court_with_jurors(&[("alice.testnet", near(5)), ("bob.testnet", near(5)), ("carol.testnet", near(5))]);
    call_as("market.testnet", NearToken::from_millinear(500), 0);

    court.create_dispute("Q".into(), outcomes());
}

#[test]
#[should_panic(expected = "A dispute needs 2 to 8 outcomes")]
fn create_dispute_refuses_a_single_outcome() {
    let mut court = court_with_jurors(&[("alice.testnet", near(5)), ("bob.testnet", near(5)), ("carol.testnet", near(5))]);
    call_as("market.testnet", near(2), 0);

    court.create_dispute("Q".into(), vec!["Yes".into()]);
}

#[test]
#[should_panic(expected = "Only jurors drawn for this round can vote")]
fn commit_refuses_an_account_that_was_not_drawn() {
    let (mut court, id) = three_juror_dispute();

    commit(&mut court, id, "mallory.testnet", YES);
}

#[test]
#[should_panic(expected = "Voting has closed")]
fn commit_refuses_once_voting_has_closed() {
    let (mut court, id) = three_juror_dispute();
    call_as("alice.testnet", zero(), VOTING_CLOSED);

    court.commit(id, Base64VecU8(commitment_hash(YES, &salt("alice.testnet"), &account("alice.testnet"))));
}

#[test]
#[should_panic(expected = "Votes can be revealed once voting closes")]
fn reveal_refuses_while_voting_is_open() {
    let (mut court, id) = three_juror_dispute();
    commit(&mut court, id, "alice.testnet", YES);
    call_as("alice.testnet", zero(), VOTING_CLOSED - 1);

    court.reveal(id, YES, Base64VecU8(salt("alice.testnet")));
}

#[test]
#[should_panic(expected = "This does not match your commitment")]
fn reveal_refuses_a_different_vote() {
    let (mut court, id) = three_juror_dispute();
    commit(&mut court, id, "alice.testnet", YES);

    reveal(&mut court, id, "alice.testnet", NO);
}

#[test]
#[should_panic(expected = "This does not match your commitment")]
fn reveal_refuses_a_commitment_copied_from_another_juror() {
    let (mut court, id) = three_juror_dispute();
    commit(&mut court, id, "alice.testnet", YES);
    let alices = court.dispute(id).unwrap().rounds[0].commitments.clone();
    let alice_seat = court.dispute(id).unwrap().rounds[0].jurors.iter().position(|j| j == &account("alice.testnet"));
    call_as("bob.testnet", zero(), 0);
    court.commit(id, alices[alice_seat.unwrap()].clone().unwrap());
    reveal(&mut court, id, "alice.testnet", YES);
    call_as("bob.testnet", zero(), VOTING_CLOSED);

    court.reveal(id, YES, Base64VecU8(salt("alice.testnet")));
}

#[test]
#[should_panic(expected = "Revealing has closed")]
fn reveal_refuses_once_revealing_has_closed() {
    let (mut court, id) = three_juror_dispute();
    commit(&mut court, id, "alice.testnet", YES);
    call_as("alice.testnet", zero(), REVEAL_CLOSED);

    court.reveal(id, YES, Base64VecU8(salt("alice.testnet")));
}

#[test]
#[should_panic(expected = "Only 4.00 NEAR is available")]
fn a_drawn_juror_cannot_ask_back_locked_stake() {
    let (mut court, _) = three_juror_dispute();
    call_as("alice.testnet", zero(), 0);

    court.request_unstake(near(5));
}
