use near_sdk::json_types::U64;
use near_sdk::mock::MockAction;
use near_sdk::test_utils::get_created_receipts;
use near_sdk::{NearToken, PromiseError};

use super::*;
use crate::test_utils::*;
use crate::Appeal;

const JURORS: [&str; 10] = [
    "j0.testnet", "j1.testnet", "j2.testnet", "j3.testnet", "j4.testnet",
    "j5.testnet", "j6.testnet", "j7.testnet", "j8.testnet", "j9.testnet",
];

/// Ten jurors with 5 NEAR each and a dispute with a 3 NEAR deposit, so appeals can draw 7.
fn ten_juror_dispute() -> (Court, U64) {
    let mut court = court_with_jurors(&JURORS.map(|name| (name, near(5))));
    call_as("market.testnet", near(3), 0);
    let id = court.create_dispute("Did it rain in Lagos on Nov 1?".into(), outcomes());
    (court, id)
}

/// Votes in the current round by seat; None means the juror at that seat never votes.
fn vote_round(court: &mut Court, id: U64, votes: &[Option<u8>]) {
    let jurors = court.dispute(id).unwrap().rounds.last().unwrap().jurors.clone();
    for (juror, choice) in jurors.iter().zip(votes) {
        if let Some(outcome) = choice {
            vote(court, id, juror.as_str(), *outcome);
        }
    }
}

fn last_tally_time(court: &Court, id: U64) -> u64 {
    court.dispute(id).unwrap().rounds.last().unwrap().reveal_ends.0
}

fn appeal(court: &mut Court, id: U64, by: &str, outcome: u8, deposit: NearToken) -> Dispute {
    let now = last_tally_time(court, id);
    call_as(by, deposit, now);
    court.appeal(id, outcome)
}

/// Finalizes as soon as the last round can no longer be appealed.
fn finalize(court: &mut Court, id: U64) -> Dispute {
    let round = court.dispute(id).unwrap().rounds.last().cloned().unwrap();
    call_as("anyone.testnet", zero(), round.appeal_ends.unwrap().0);
    let _ = court.finalize(id);
    court.dispute(id).unwrap()
}

fn receipts_debug() -> Vec<(String, String)> {
    get_created_receipts()
        .into_iter()
        .map(|receipt| (receipt.receiver_id.to_string(), format!("{:?}", receipt.actions)))
        .collect()
}

/// Whether a receipt created by the last call runs `method` on `receiver`.
fn called(receiver: &str, method: &str) -> bool {
    get_created_receipts().into_iter().any(|receipt| {
        receipt.receiver_id.as_str() == receiver
            && receipt.actions.iter().any(|action| {
                matches!(action, MockAction::FunctionCallWeight { method_name, .. } if method_name == method.as_bytes())
            })
    })
}

fn all_names() -> [&'static str; 3] {
    ["alice.testnet", "bob.testnet", "carol.testnet"]
}

#[test]
fn tally_counts_the_votes_and_opens_the_appeal_window() {
    let (mut court, id) = three_juror_dispute();
    vote(&mut court, id, "alice.testnet", YES);
    vote(&mut court, id, "bob.testnet", YES);
    vote(&mut court, id, "carol.testnet", NO);

    let dispute = tally(&mut court, id);

    let round = &dispute.rounds[0];
    assert_eq!(
        (round.tallied, round.outcome, round.appeal_ends, dispute.finalized),
        (true, Some(YES), Some(U64(REVEAL_CLOSED + APPEAL_NS)), false)
    );
    // Nobody is paid yet; every lock is held until the ruling is final.
    for name in all_names() {
        assert_eq!((staked(&court, name), locked(&court, name)), (near(5), near(1)), "{name}");
    }
}

#[test]
fn tally_leaves_a_tie_without_an_outcome() {
    let (mut court, id) = three_juror_dispute();
    vote(&mut court, id, "alice.testnet", YES);
    vote(&mut court, id, "bob.testnet", NO);

    assert_eq!(tally(&mut court, id).rounds[0].outcome, None);
}

#[test]
#[should_panic(expected = "Revealing closes at")]
fn tally_refuses_while_revealing_is_open() {
    let (mut court, id) = three_juror_dispute();
    call_as("anyone.testnet", zero(), REVEAL_CLOSED - 1);

    court.tally(id);
}

#[test]
#[should_panic(expected = "This round is already counted")]
fn tally_refuses_a_round_it_already_counted() {
    let (mut court, id) = three_juror_dispute();
    tally(&mut court, id);

    tally(&mut court, id);
}

#[test]
#[should_panic(expected = "Voting for this round is over")]
fn commit_refuses_a_tallied_round() {
    let (mut court, id) = three_juror_dispute();
    tally(&mut court, id);

    commit(&mut court, id, "alice.testnet", YES);
}

#[test]
#[should_panic(expected = "Appeals close at")]
fn finalize_refuses_while_appeals_are_open() {
    let (mut court, id) = three_juror_dispute();
    tally(&mut court, id);
    call_as("anyone.testnet", zero(), REVEAL_CLOSED + APPEAL_NS - 1);

    let _ = court.finalize(id);
}

#[test]
#[should_panic(expected = "The current round has not been counted yet")]
fn finalize_refuses_an_uncounted_round() {
    let (mut court, id) = three_juror_dispute();
    call_as("anyone.testnet", zero(), REVEAL_CLOSED + APPEAL_NS);

    let _ = court.finalize(id);
}

#[test]
fn finalize_pays_the_majority_from_the_minoritys_lock_and_the_fee() {
    let (mut court, id) = three_juror_dispute();
    vote(&mut court, id, "alice.testnet", YES);
    vote(&mut court, id, "bob.testnet", YES);
    vote(&mut court, id, "carol.testnet", NO);
    let fee = tally(&mut court, id).rounds[0].fee;

    let dispute = finalize(&mut court, id);

    assert_eq!((dispute.finalized, dispute.ruling), (true, Some(YES)));
    // The fee and Carol's 1 NEAR lock, split between Alice and Bob.
    assert_eq!(total_staked(&court, &["alice.testnet", "bob.testnet"]), near(11).as_yoctonear() + fee.as_yoctonear());
    assert_eq!(staked(&court, "carol.testnet"), near(4));
    for name in all_names() {
        assert_eq!(locked(&court, name), zero(), "{name}");
    }
}

#[test]
fn finalize_treats_a_missed_reveal_like_a_minority_vote() {
    let (mut court, id) = three_juror_dispute();
    vote(&mut court, id, "alice.testnet", NO);
    vote(&mut court, id, "bob.testnet", NO);
    commit(&mut court, id, "carol.testnet", NO);
    let fee = tally(&mut court, id).rounds[0].fee;

    assert_eq!(finalize(&mut court, id).ruling, Some(NO));
    assert_eq!(total_staked(&court, &["alice.testnet", "bob.testnet"]), near(11).as_yoctonear() + fee.as_yoctonear());
    assert_eq!(staked(&court, "carol.testnet"), near(4));
}

#[test]
fn finalize_loses_no_yoctonear_to_rounding() {
    let (mut court, id) = three_juror_dispute();
    for name in all_names() {
        vote(&mut court, id, name, YES);
    }
    let fee = tally(&mut court, id).rounds[0].fee;

    finalize(&mut court, id);

    assert_ne!(fee.as_yoctonear() % 3, 0);
    assert_eq!(total_staked(&court, &all_names()), near(15).as_yoctonear() + fee.as_yoctonear());
}

#[test]
fn a_tie_nobody_appeals_ends_with_no_ruling_and_pays_those_who_revealed() {
    let (mut court, id) = three_juror_dispute();
    vote(&mut court, id, "alice.testnet", YES);
    vote(&mut court, id, "bob.testnet", NO);
    let fee = tally(&mut court, id).rounds[0].fee;

    let dispute = finalize(&mut court, id);

    assert_eq!((dispute.finalized, dispute.ruling), (true, None));
    assert_eq!(total_staked(&court, &["alice.testnet", "bob.testnet"]), near(11).as_yoctonear() + fee.as_yoctonear());
    assert_eq!(staked(&court, "carol.testnet"), near(4));
}

#[test]
fn finalize_returns_the_money_to_the_arbitrable_when_nobody_voted() {
    let (mut court, id) = three_juror_dispute();
    let fee = tally(&mut court, id).rounds[0].fee;

    finalize(&mut court, id);

    for name in all_names() {
        assert_eq!(staked(&court, name), near(4), "{name}");
    }
    let refund = fee.saturating_add(near(3)).as_yoctonear().to_string();
    assert!(receipts_debug().iter().any(|(to, actions)| to == "market.testnet" && actions.contains(&refund)));
}

#[test]
fn finalize_sends_the_ruling_to_the_arbitrable() {
    let (mut court, id) = three_juror_dispute();
    vote(&mut court, id, "alice.testnet", YES);
    tally(&mut court, id);

    finalize(&mut court, id);

    assert!(called("market.testnet", "on_ruling"));
}

#[test]
#[should_panic(expected = "This dispute has a final ruling")]
fn finalize_refuses_a_final_dispute() {
    let (mut court, id) = three_juror_dispute();
    tally(&mut court, id);
    finalize(&mut court, id);

    finalize(&mut court, id);
}

#[test]
fn on_ruling_delivered_records_a_successful_delivery() {
    let (mut court, id) = three_juror_dispute();
    tally(&mut court, id);
    finalize(&mut court, id);
    call_as("court.testnet", zero(), REVEAL_CLOSED + APPEAL_NS);

    assert!(court.on_ruling_delivered(id, Ok(())));
    assert!(court.dispute(id).unwrap().ruling_delivered);
}

#[test]
fn deliver_ruling_sends_it_again_after_a_failed_delivery() {
    let (mut court, id) = three_juror_dispute();
    tally(&mut court, id);
    finalize(&mut court, id);
    call_as("court.testnet", zero(), REVEAL_CLOSED + APPEAL_NS);
    assert!(!court.on_ruling_delivered(id, Err(PromiseError::Failed)));
    call_as("anyone.testnet", zero(), REVEAL_CLOSED + APPEAL_NS);

    let _ = court.deliver_ruling(id);

    assert!(called("market.testnet", "on_ruling"));
}

#[test]
#[should_panic(expected = "The ruling was already delivered")]
fn deliver_ruling_refuses_once_delivered() {
    let (mut court, id) = three_juror_dispute();
    tally(&mut court, id);
    finalize(&mut court, id);
    call_as("court.testnet", zero(), REVEAL_CLOSED + APPEAL_NS);
    court.on_ruling_delivered(id, Ok(()));

    let _ = court.deliver_ruling(id);
}

#[test]
#[should_panic(expected = "The ruling is not final yet")]
fn deliver_ruling_refuses_before_the_ruling_is_final() {
    let (mut court, id) = three_juror_dispute();

    let _ = court.deliver_ruling(id);
}

#[test]
fn appeal_draws_a_larger_jury_paid_for_by_the_appellant() {
    let (mut court, id) = ten_juror_dispute();
    vote_round(&mut court, id, &[Some(YES), Some(YES), Some(NO)]);
    tally(&mut court, id);

    let dispute = appeal(&mut court, id, "dan.testnet", NO, near(4));

    let next = &dispute.rounds[1];
    assert_eq!(next.jurors.len(), 7);
    // 7 jurors × 0.5 NEAR; the rest of the 4 NEAR deposit, less the round's storage, also goes to them.
    assert!(next.fee >= NearToken::from_millinear(3_500) && next.fee < near(4));
    assert_eq!(
        dispute.rounds[0].appeal,
        Some(Appeal {
            by: account("dan.testnet"),
            outcome: NO,
            paid: next.fee,
        })
    );
}

#[test]
fn appeal_can_break_a_tie() {
    let (mut court, id) = ten_juror_dispute();
    vote_round(&mut court, id, &[Some(YES), Some(NO), None]);
    tally(&mut court, id);

    assert_eq!(appeal(&mut court, id, "dan.testnet", YES, near(4)).rounds.len(), 2);
}

#[test]
#[should_panic(expected = "Appeals closed at")]
fn appeal_refuses_once_appeals_have_closed() {
    let (mut court, id) = ten_juror_dispute();
    vote_round(&mut court, id, &[Some(YES), Some(YES), Some(NO)]);
    tally(&mut court, id);
    call_as("dan.testnet", near(4), REVEAL_CLOSED + APPEAL_NS);

    court.appeal(id, NO);
}

#[test]
#[should_panic(expected = "Appeal for an outcome other than the round's")]
fn appeal_refuses_to_back_the_rounds_own_outcome() {
    let (mut court, id) = ten_juror_dispute();
    vote_round(&mut court, id, &[Some(YES), Some(YES), Some(NO)]);
    tally(&mut court, id);

    appeal(&mut court, id, "dan.testnet", YES, near(4));
}

#[test]
#[should_panic(expected = "Attach at least 3.50 NEAR plus")]
fn appeal_refuses_a_deposit_that_does_not_cover_the_jury_and_storage() {
    let (mut court, id) = ten_juror_dispute();
    vote_round(&mut court, id, &[Some(YES), Some(YES), Some(NO)]);
    tally(&mut court, id);

    appeal(&mut court, id, "dan.testnet", NO, NearToken::from_millinear(3_500));
}

#[test]
#[should_panic(expected = "The current round has not been counted yet")]
fn appeal_refuses_while_the_appeal_round_is_open() {
    let (mut court, id) = ten_juror_dispute();
    vote_round(&mut court, id, &[Some(YES), Some(YES), Some(NO)]);
    tally(&mut court, id);
    appeal(&mut court, id, "dan.testnet", NO, near(4));

    appeal(&mut court, id, "erin.testnet", YES, near(4));
}

#[test]
#[should_panic(expected = "No more appeals: 7 jurors is the largest jury")]
fn appeal_refuses_past_the_largest_jury() {
    let (mut court, id) = ten_juror_dispute();
    vote_round(&mut court, id, &[Some(YES), Some(YES), Some(NO)]);
    tally(&mut court, id);
    appeal(&mut court, id, "dan.testnet", NO, near(4));
    vote_round(&mut court, id, &[Some(NO); 7]);
    tally(&mut court, id);

    appeal(&mut court, id, "erin.testnet", YES, near(8));
}

#[test]
fn an_overturned_ruling_repays_the_appellant_from_the_jurors_who_got_it_wrong() {
    let (mut court, id) = ten_juror_dispute();
    vote_round(&mut court, id, &[Some(YES), Some(YES), Some(NO)]);
    let first_fee = tally(&mut court, id).rounds[0].fee;
    let paid = appeal(&mut court, id, "dan.testnet", NO, near(4)).rounds[1].fee;
    vote_round(&mut court, id, &[Some(NO); 7]);
    tally(&mut court, id);

    let dispute = finalize(&mut court, id);

    assert_eq!(dispute.ruling, Some(NO));
    let repaid = paid.as_yoctonear().to_string();
    assert!(receipts_debug().iter().any(|(to, actions)| to == "dan.testnet" && actions.contains(&repaid)));
    // Dan's deposit paid the second jury and came back from the first jury's wrong votes,
    // so the jurors together gained exactly the first round's fee.
    assert_eq!(total_staked(&court, &JURORS), near(50).as_yoctonear() + first_fee.as_yoctonear());
    for name in JURORS {
        assert_eq!(locked(&court, name), zero(), "{name}");
    }
}

#[test]
fn the_largest_jury_can_be_finalized_as_soon_as_it_is_counted() {
    let (mut court, id) = ten_juror_dispute();
    vote_round(&mut court, id, &[Some(YES), Some(YES), Some(NO)]);
    tally(&mut court, id);
    appeal(&mut court, id, "dan.testnet", NO, near(4));
    vote_round(&mut court, id, &[Some(NO); 7]);

    let dispute = tally(&mut court, id);

    let last = dispute.rounds.last().unwrap();
    assert_eq!(last.appeal_ends, Some(last.reveal_ends));
}

#[test]
fn finalize_survives_a_last_round_juror_who_loses_their_whole_stake() {
    // Round 0 is unanimous for YES and overturned, so its pot goes to the last round's NO voters,
    // while a YES voter in the last round who staked exactly one lock is removed.
    let names = [
        "j0.testnet", "j1.testnet", "j2.testnet", "j3.testnet", "j4.testnet",
        "j5.testnet", "j6.testnet", "j7.testnet", "j8.testnet", "j9.testnet",
    ];
    let mut court = court_with_jurors(&names.map(|name| (name, near(1))));
    call_as("market.testnet", near(3), 0);
    let id = court.create_dispute("Q".into(), outcomes());
    vote_round(&mut court, id, &[Some(YES); 3]);
    tally(&mut court, id);
    appeal(&mut court, id, "dan.testnet", NO, near(4));
    vote_round(&mut court, id, &[Some(NO), Some(NO), Some(NO), Some(NO), Some(YES), Some(YES), Some(YES)]);
    tally(&mut court, id);

    let dispute = finalize(&mut court, id);

    assert_eq!((dispute.finalized, dispute.ruling), (true, Some(NO)));
    let loser = dispute.rounds[1].jurors[4].clone();
    assert_eq!(court.juror(loser), None);
}

#[test]
fn an_appeal_jury_has_none_of_the_earlier_jurors() {
    let (mut court, id) = ten_juror_dispute();
    vote_round(&mut court, id, &[Some(YES), Some(YES), Some(NO)]);
    tally(&mut court, id);

    let dispute = appeal(&mut court, id, "dan.testnet", NO, near(4));

    assert!(dispute.rounds[1].jurors.iter().all(|juror| !dispute.rounds[0].jurors.contains(juror)));
}
