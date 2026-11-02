use super::*;
use near_sdk::{test_utils::VMContextBuilder, testing_env};

fn account(name: &str) -> AccountId {
    name.parse().unwrap()
}
fn context(name: &str, deposit: u128, ms: u64) {
    testing_env!(VMContextBuilder::new()
        .current_account_id(account("events.testnet"))
        .predecessor_account_id(account(name))
        .attached_deposit(NearToken::from_yoctonear(deposit))
        .block_timestamp(ms * 1_000_000)
        .prepaid_gas(Gas::from_tgas(300))
        .build());
}
fn near(n: u128) -> u128 {
    NearToken::from_near(n).as_yoctonear()
}
fn setup(split: u32, capacity: u32) -> (Events, U64) {
    context("host.testnet", 0, 0);
    let mut contract = Events::new();
    context("host.testnet", contract.storage_fees().event.0, 0);
    let id = contract.create_event(
        "Builders meetup".into(),
        "Bring your project".into(),
        "Lagos".into(),
        capacity,
        U128(near(1)),
        split,
        U64(100),
        U64(200),
        U64(300),
    );
    (contract, id)
}
fn register(c: &mut Events, id: U64, who: &str) {
    context(who, near(1) + c.storage_fees().registration.0, 1);
    c.register(id);
}
fn checkin(c: &mut Events, id: U64, who: &str) {
    context("host.testnet", 0, 200);
    c.check_in(id, account(who));
}
fn finalize(c: &mut Events, id: U64) {
    context("anyone.testnet", 0, 300);
    c.finalize(id);
}

#[test]
fn ten_deposits_eight_attendees_split_exactly() {
    let (mut c, id) = setup(5000, 10);
    for n in 0..10 {
        register(&mut c, id, &format!("guest{n}.testnet"));
    }
    for n in 0..8 {
        checkin(&mut c, id, &format!("guest{n}.testnet"));
    }
    finalize(&mut c, id);
    assert_eq!(
        c.claimable(id, account("guest0.testnet")).0,
        near(1) + near(1) / 8
    );
    assert_eq!(c.claimable(id, account("guest9.testnet")).0, 0);
    assert_eq!(c.get(id).host_claimable.0, near(1));
    assert_eq!(
        c.get(id).attendee_payout.0 * 8 + c.get(id).host_claimable.0,
        near(10)
    );
}

#[test]
fn split_boundaries_and_rounding_conserve_the_pool() {
    // Separate mocked storage namespaces for independent cases.
    for split in [0, 1, 5000, 9999, 10000] {
        context("host.testnet", 0, 0);
        let mut c = Events {
            events: Vector::new(format!("e{split}").as_bytes()),
            registrations: LookupMap::new(format!("r{split}").as_bytes()),
        };
        context("host.testnet", c.storage_fees().event.0, 0);
        let id = c.create_event(
            "A".into(),
            "".into(),
            "B".into(),
            4,
            U128(near(1)),
            split,
            U64(100),
            U64(200),
            U64(300),
        );
        for name in ["a.testnet", "b.testnet", "c.testnet", "d.testnet"] {
            register(&mut c, id, name);
        }
        for name in ["a.testnet", "b.testnet", "c.testnet"] {
            checkin(&mut c, id, name);
        }
        finalize(&mut c, id);
        let e = c.get(id);
        assert_eq!(e.attendee_payout.0 * 3 + e.host_claimable.0, near(4));
    }
}

#[test]
fn nobody_attending_refunds_everyone() {
    let (mut c, id) = setup(5000, 2);
    register(&mut c, id, "a.testnet");
    register(&mut c, id, "b.testnet");
    finalize(&mut c, id);
    assert_eq!(c.claimable(id, account("a.testnet")).0, near(1));
    assert_eq!(c.get(id).host_claimable.0, 0);
}

#[test]
fn cancelled_event_refunds_without_waiting() {
    let (mut c, id) = setup(5000, 2);
    register(&mut c, id, "a.testnet");
    context("host.testnet", 0, 50);
    c.cancel_event(id);
    assert_eq!(c.claimable(id, account("a.testnet")).0, near(1));
}

#[test]
fn cancellation_releases_seat_and_refund_survives_failed_transfer() {
    let (mut c, id) = setup(5000, 1);
    register(&mut c, id, "a.testnet");
    context("a.testnet", 0, 50);
    c.cancel_registration(id);
    assert_eq!(c.get(id).registered, 0);
    assert!(c.attendees(id, None, None).is_empty());
    register(&mut c, id, "b.testnet");
    context("a.testnet", 0, 60);
    let _ = c.claim(id);
    assert_eq!(c.claimable(id, account("a.testnet")).0, 0);
    context("events.testnet", 0, 60);
    assert!(!c.on_payout(
        id,
        account("a.testnet"),
        U128(near(1)),
        false,
        Err(PromiseError::Failed)
    ));
    assert_eq!(c.claimable(id, account("a.testnet")).0, near(1));
    context("a.testnet", 0, 61);
    let _ = c.claim(id);
    context("events.testnet", 0, 61);
    assert!(c.on_payout(id, account("a.testnet"), U128(near(1)), false, Ok(())));
    assert_eq!(c.claimable(id, account("a.testnet")).0, 0);
}

#[test]
fn failed_settlement_claim_restores_only_one_entitlement() {
    let (mut c, id) = setup(5000, 2);
    register(&mut c, id, "a.testnet");
    register(&mut c, id, "b.testnet");
    checkin(&mut c, id, "a.testnet");
    finalize(&mut c, id);
    let amount = c.claimable(id, account("a.testnet"));
    context("a.testnet", 0, 301);
    let _ = c.claim(id);
    context("events.testnet", 0, 301);
    c.on_payout(
        id,
        account("a.testnet"),
        amount,
        false,
        Err(PromiseError::Failed),
    );
    assert_eq!(c.claimable(id, account("a.testnet")), amount);
}

#[test]
fn failed_host_payout_is_retryable() {
    let (mut c, id) = setup(5000, 2);
    register(&mut c, id, "a.testnet");
    register(&mut c, id, "b.testnet");
    checkin(&mut c, id, "a.testnet");
    finalize(&mut c, id);
    let amount = c.get(id).host_claimable;
    context("host.testnet", 0, 301);
    let _ = c.claim_host(id);
    context("events.testnet", 0, 301);
    c.on_payout(
        id,
        account("host.testnet"),
        amount,
        true,
        Err(PromiseError::Failed),
    );
    assert_eq!(c.get(id).host_claimable, amount);
}

#[test]
#[should_panic(expected = "Already registered")]
fn duplicate_registration_rejected() {
    let (mut c, id) = setup(5000, 2);
    register(&mut c, id, "a.testnet");
    register(&mut c, id, "a.testnet");
}
#[test]
#[should_panic(expected = "Event is full")]
fn capacity_enforced() {
    let (mut c, id) = setup(5000, 1);
    register(&mut c, id, "a.testnet");
    register(&mut c, id, "b.testnet");
}
#[test]
#[should_panic(expected = "Registration is closed")]
fn cutoff_is_exclusive() {
    let (mut c, id) = setup(5000, 2);
    context("a.testnet", near(1) + c.storage_fees().registration.0, 100);
    c.register(id);
}
#[test]
#[should_panic(expected = "Cancellation cutoff")]
fn cannot_cancel_at_cutoff() {
    let (mut c, id) = setup(5000, 1);
    register(&mut c, id, "a.testnet");
    context("a.testnet", 0, 100);
    c.cancel_registration(id);
}
#[test]
#[should_panic(expected = "Only the organizer")]
fn outsider_cannot_check_in() {
    let (mut c, id) = setup(5000, 1);
    register(&mut c, id, "a.testnet");
    context("a.testnet", 0, 200);
    c.check_in(id, account("a.testnet"));
}
#[test]
#[should_panic(expected = "Only the organizer")]
fn outsider_cannot_cancel_event() {
    let (mut c, id) = setup(5000, 1);
    context("a.testnet", 0, 1);
    c.cancel_event(id);
}
#[test]
#[should_panic(expected = "Cannot cancel")]
fn host_cannot_cancel_when_checkin_opens() {
    let (mut c, id) = setup(5000, 1);
    context("host.testnet", 0, 200);
    c.cancel_event(id);
}
#[test]
#[should_panic(expected = "already checked in")]
fn duplicate_checkin_rejected() {
    let (mut c, id) = setup(5000, 1);
    register(&mut c, id, "a.testnet");
    checkin(&mut c, id, "a.testnet");
    checkin(&mut c, id, "a.testnet");
}
#[test]
#[should_panic(expected = "Outside check-in")]
fn checkin_end_is_exclusive() {
    let (mut c, id) = setup(5000, 1);
    register(&mut c, id, "a.testnet");
    context("host.testnet", 0, 300);
    c.check_in(id, account("a.testnet"));
}
#[test]
#[should_panic(expected = "not ready")]
fn early_settlement_rejected() {
    let (mut c, id) = setup(5000, 1);
    context("host.testnet", 0, 299);
    c.finalize(id);
}
#[test]
#[should_panic(expected = "Nothing to claim")]
fn unresolved_claim_rejected() {
    let (mut c, id) = setup(5000, 1);
    register(&mut c, id, "a.testnet");
    context("a.testnet", 0, 5);
    let _ = c.claim(id);
}
#[test]
#[should_panic(expected = "already in flight")]
fn in_flight_double_claim_rejected() {
    let (mut c, id) = setup(5000, 1);
    register(&mut c, id, "a.testnet");
    finalize(&mut c, id);
    context("a.testnet", 0, 301);
    let _ = c.claim(id);
    let _ = c.claim(id);
}
#[test]
#[should_panic(expected = "Attach exactly")]
fn storage_fee_is_separate() {
    let (mut c, id) = setup(5000, 1);
    context("a.testnet", near(1), 1);
    c.register(id);
}
#[test]
fn max_sized_event_storage_fits_reserve() {
    let (mut c, id) = setup(5000, 200);
    for n in 0..200 {
        register(&mut c, id, &format!("guest{n}.testnet"));
    }
    assert_eq!(c.get(id).registered, 200);
    assert_eq!(c.attendees(id, Some(100), Some(100)).len(), 100);
}
