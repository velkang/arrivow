//! Attendance deposits. The organizer reports attendance; this contract enforces the money rules.
use near_sdk::json_types::{U128, U64};
use near_sdk::store::{LookupMap, Vector};
use near_sdk::{
    env, near, require, AccountId, Gas, NearToken, PanicOnDefault, Promise, PromiseError,
};

const EVENT_BYTES: u128 = 30_000;
const REGISTRATION_BYTES: u128 = 1_024;
const CALLBACK_GAS: Gas = Gas::from_tgas(15);

#[near(serializers = [json, borsh])]
#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    Open,
    Cancelled,
    Settled,
}

#[near(serializers = [json, borsh])]
#[derive(Clone, Debug)]
pub struct Event {
    pub organizer: AccountId,
    pub title: String,
    pub description: String,
    pub location: String,
    pub capacity: u32,
    pub deposit: U128,
    pub bonus_bps: u32,
    /// Unix milliseconds. Keep browser dates exact and comfortably below u64 limits.
    pub registration_closes: U64,
    pub checkin_opens: U64,
    pub checkin_closes: U64,
    pub status: Status,
    pub registered: u32,
    pub attended: u32,
    pub attendee_payout: U128,
    pub host_claimable: U128,
    pub host_pending: U128,
    /// Active registrations only. Bounded by capacity; settlement never scans this list.
    pub accounts: Vec<AccountId>,
}

#[near(serializers = [json, borsh])]
#[derive(Clone, Debug, Default)]
pub struct Registration {
    pub active: bool,
    pub checked_in: bool,
    pub claimed: bool,
    pub credit: U128,
    pub pending: U128,
}

#[near(serializers = [json])]
pub struct StorageFees {
    pub event: U128,
    pub registration: U128,
}

#[near(contract_state)]
#[derive(PanicOnDefault)]
pub struct Events {
    events: Vector<Event>,
    registrations: LookupMap<(u64, AccountId), Registration>,
}

#[near]
impl Events {
    #[init]
    pub fn new() -> Self {
        Self {
            events: Vector::new(b"e"),
            registrations: LookupMap::new(b"r"),
        }
    }

    /// Fixed non-refundable storage reserves, separate from attendance deposits.
    /// The event fee reserves space for its maximum guest list and settlement fields.
    pub fn storage_fees(&self) -> StorageFees {
        let byte = env::storage_byte_cost().as_yoctonear();
        StorageFees {
            event: U128(byte * EVENT_BYTES),
            registration: U128(byte * REGISTRATION_BYTES),
        }
    }

    #[payable]
    pub fn create_event(
        &mut self,
        title: String,
        description: String,
        location: String,
        capacity: u32,
        deposit: U128,
        bonus_bps: u32,
        registration_closes: U64,
        checkin_opens: U64,
        checkin_closes: U64,
    ) -> U64 {
        require!(
            !title.trim().is_empty() && title.len() <= 120,
            "Title must be 1–120 bytes"
        );
        require!(
            description.len() <= 2_000,
            "Description must be at most 2000 bytes"
        );
        require!(
            !location.trim().is_empty() && location.len() <= 300,
            "Location must be 1–300 bytes"
        );
        require!((1..=200).contains(&capacity), "Capacity must be 1–200");
        // Bounds keep every pool and basis-point calculation within u128.
        require!(
            deposit.0 > 0 && deposit.0 <= NearToken::from_near(1_000_000).as_yoctonear(),
            "Deposit must be positive and at most 1000000 NEAR"
        );
        require!(
            bonus_bps <= 10_000,
            "Bonus share must be 0–10000 basis points"
        );
        require!(
            registration_closes.0 > now()
                && registration_closes.0 <= checkin_opens.0
                && checkin_opens.0 < checkin_closes.0,
            "Order deadlines: future registration cutoff, check-in opens, check-in closes"
        );
        exact_deposit(self.storage_fees().event.0);
        let id = self.events.len() as u64;
        self.events.push(Event {
            organizer: env::predecessor_account_id(),
            title,
            description,
            location,
            capacity,
            deposit,
            bonus_bps,
            registration_closes,
            checkin_opens,
            checkin_closes,
            status: Status::Open,
            registered: 0,
            attended: 0,
            attendee_payout: U128(0),
            host_claimable: U128(0),
            host_pending: U128(0),
            accounts: vec![],
        });
        U64(id)
    }

    #[payable]
    pub fn register(&mut self, event_id: U64) {
        let mut event = self.get(event_id);
        require!(
            event.status == Status::Open && now() < event.registration_closes.0,
            "Registration is closed"
        );
        require!(event.registered < event.capacity, "Event is full");
        let account = env::predecessor_account_id();
        let previous = self
            .registration(event_id, account.clone())
            .unwrap_or_default();
        require!(
            !previous.active && previous.credit.0 == 0 && previous.pending.0 == 0,
            "Already registered, or claim the previous refund first"
        );
        exact_deposit(event.deposit.0 + self.storage_fees().registration.0);
        self.registrations.insert(
            (event_id.0, account.clone()),
            Registration {
                active: true,
                ..Registration::default()
            },
        );
        event.accounts.push(account);
        event.registered += 1;
        self.save(event_id, event);
    }

    /// Frees the seat immediately. The attendance deposit is available through claim().
    pub fn cancel_registration(&mut self, event_id: U64) {
        let mut event = self.get(event_id);
        require!(
            event.status == Status::Open && now() < event.registration_closes.0,
            "Cancellation cutoff has passed"
        );
        let account = env::predecessor_account_id();
        let mut registration = self.reg(event_id, &account);
        require!(registration.active, "No active registration");
        registration.active = false;
        registration.claimed = true;
        registration.credit = event.deposit;
        event.registered -= 1;
        event.accounts.retain(|id| id != &account);
        self.registrations
            .insert((event_id.0, account), registration);
        self.save(event_id, event);
    }

    pub fn check_in(&mut self, event_id: U64, account_id: AccountId) {
        let mut event = self.get(event_id);
        host_only(&event);
        require!(
            event.status == Status::Open
                && now() >= event.checkin_opens.0
                && now() < event.checkin_closes.0,
            "Outside check-in window"
        );
        let mut registration = self.reg(event_id, &account_id);
        require!(
            registration.active && !registration.checked_in,
            "Not registered or already checked in"
        );
        registration.checked_in = true;
        event.attended += 1;
        self.registrations
            .insert((event_id.0, account_id), registration);
        self.save(event_id, event);
    }

    pub fn cancel_event(&mut self, event_id: U64) {
        let mut event = self.get(event_id);
        host_only(&event);
        require!(
            event.status == Status::Open && now() < event.checkin_opens.0,
            "Cannot cancel after check-in opens"
        );
        event.status = Status::Cancelled;
        self.save(event_id, event);
    }

    /// Permissionless settlement from counters, without scanning registration records.
    pub fn finalize(&mut self, event_id: U64) {
        let mut event = self.get(event_id);
        require!(
            event.status == Status::Open && now() >= event.checkin_closes.0,
            "Event is not ready to settle"
        );
        if event.attended == 0 {
            event.attendee_payout = event.deposit;
        } else {
            let forfeited = event.deposit.0 * (event.registered - event.attended) as u128;
            let per_guest = (forfeited * event.bonus_bps as u128 / 10_000) / event.attended as u128;
            event.attendee_payout = U128(event.deposit.0 + per_guest);
            event.host_claimable = U128(forfeited - per_guest * event.attended as u128);
        }
        event.status = Status::Settled;
        self.save(event_id, event);
    }

    pub fn claimable(&self, event_id: U64, account_id: AccountId) -> U128 {
        let event = self.get(event_id);
        let Some(registration) = self.registration(event_id, account_id) else {
            return U128(0);
        };
        U128(registration.credit.0 + entitlement(&event, &registration))
    }

    pub fn claim(&mut self, event_id: U64) -> Promise {
        require!(
            env::prepaid_gas() >= Gas::from_tgas(30),
            "Attach at least 30 Tgas for payout recovery"
        );
        let account = env::predecessor_account_id();
        let event = self.get(event_id);
        let mut registration = self.reg(event_id, &account);
        require!(registration.pending.0 == 0, "A payout is already in flight");
        let amount = registration.credit.0 + entitlement(&event, &registration);
        require!(amount > 0, "Nothing to claim");
        registration.credit = U128(0);
        registration.claimed = true;
        registration.pending = U128(amount);
        self.registrations
            .insert((event_id.0, account.clone()), registration);
        self.transfer(event_id, account, amount, false)
    }

    pub fn claim_host(&mut self, event_id: U64) -> Promise {
        require!(
            env::prepaid_gas() >= Gas::from_tgas(30),
            "Attach at least 30 Tgas for payout recovery"
        );
        let mut event = self.get(event_id);
        host_only(&event);
        let amount = event.host_claimable.0;
        require!(
            amount > 0 && event.host_pending.0 == 0,
            "Nothing to claim or payout in flight"
        );
        event.host_claimable = U128(0);
        event.host_pending = U128(amount);
        let account = event.organizer.clone();
        self.save(event_id, event);
        self.transfer(event_id, account, amount, true)
    }

    #[private]
    pub fn on_payout(
        &mut self,
        event_id: U64,
        account_id: AccountId,
        amount: U128,
        host: bool,
        #[callback_result] result: Result<(), PromiseError>,
    ) -> bool {
        if host {
            let mut event = self.get(event_id);
            require!(event.host_pending == amount, "Payout mismatch");
            event.host_pending = U128(0);
            if result.is_err() {
                event.host_claimable = amount;
            }
            self.save(event_id, event);
        } else {
            let mut registration = self.reg(event_id, &account_id);
            require!(registration.pending == amount, "Payout mismatch");
            registration.pending = U128(0);
            if result.is_err() {
                registration.credit = U128(registration.credit.0 + amount.0);
            }
            self.registrations
                .insert((event_id.0, account_id), registration);
        }
        result.is_ok()
    }

    pub fn event(&self, event_id: U64) -> Option<Event> {
        u32::try_from(event_id.0)
            .ok()
            .and_then(|id| self.events.get(id).cloned())
    }

    pub fn events(&self, from_index: Option<u32>, limit: Option<u32>) -> Vec<(U64, Event)> {
        self.events
            .iter()
            .enumerate()
            .skip(from_index.unwrap_or(0) as usize)
            .take(limit.unwrap_or(25).min(50) as usize)
            .map(|(id, event)| (U64(id as u64), event.clone()))
            .collect()
    }

    pub fn registration(&self, event_id: U64, account_id: AccountId) -> Option<Registration> {
        self.registrations.get(&(event_id.0, account_id)).cloned()
    }

    pub fn attendees(
        &self,
        event_id: U64,
        from_index: Option<u32>,
        limit: Option<u32>,
    ) -> Vec<(AccountId, Registration)> {
        self.get(event_id)
            .accounts
            .iter()
            .skip(from_index.unwrap_or(0) as usize)
            .take(limit.unwrap_or(50).min(100) as usize)
            .map(|account| (account.clone(), self.reg(event_id, account)))
            .collect()
    }
}

impl Events {
    fn get(&self, id: U64) -> Event {
        self.event(id)
            .unwrap_or_else(|| env::panic_str("Event not found"))
    }
    fn save(&mut self, id: U64, event: Event) {
        self.events.replace(id.0 as u32, event);
    }
    fn reg(&self, id: U64, account: &AccountId) -> Registration {
        self.registration(id, account.clone())
            .unwrap_or_else(|| env::panic_str("Registration not found"))
    }
    fn transfer(&self, event_id: U64, account_id: AccountId, amount: u128, host: bool) -> Promise {
        Promise::new(account_id.clone())
            .transfer(NearToken::from_yoctonear(amount))
            .then(
                Self::ext(env::current_account_id())
                    .with_static_gas(CALLBACK_GAS)
                    .on_payout(event_id, account_id, U128(amount), host),
            )
    }
}

fn now() -> u64 {
    env::block_timestamp_ms()
}
fn host_only(event: &Event) {
    require!(
        env::predecessor_account_id() == event.organizer,
        "Only the organizer can do this"
    );
}
fn exact_deposit(amount: u128) {
    require!(
        env::attached_deposit().as_yoctonear() == amount,
        format!("Attach exactly {amount} yoctoNEAR")
    );
}
fn entitlement(event: &Event, registration: &Registration) -> u128 {
    if !registration.active || registration.claimed {
        return 0;
    }
    match event.status {
        Status::Cancelled => event.deposit.0,
        Status::Settled if event.attended == 0 || registration.checked_in => {
            event.attendee_payout.0
        }
        _ => 0,
    }
}

#[cfg(test)]
mod tests;
