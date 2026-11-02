use near_sdk::json_types::U64;
use near_sdk::{env, ext_contract, near, require, AccountId, Gas, NearToken, Promise, PromiseError};

use crate::dispute::current_round;
use crate::{Appeal, Court, CourtExt, Dispute, Juror, Round};

#[cfg(test)]
mod tests;

const ON_RULING_GAS: Gas = Gas::from_tgas(20);
const ON_RULING_DELIVERED_GAS: Gas = Gas::from_tgas(5);
const NOT_COUNTED: &str = "The current round has not been counted yet";

/// What a contract that opens disputes implements to receive the final ruling.
#[ext_contract(ext_arbitrable)]
pub trait Arbitrable {
    /// `ruling` is None when the last round ended in a tie. It must return nothing, and it may
    /// be called more than once for the same dispute (a retry while a delivery is in flight),
    /// so it must ignore repeats.
    fn on_ruling(&mut self, dispute_id: U64, ruling: Option<u8>);
}

#[near]
impl Court {
    /// Counts the current round's revealed votes once revealing has closed and opens its
    /// appeal window. Stake stays locked; nobody is paid until the ruling is final.
    pub fn tally(&mut self, dispute_id: U64) -> Dispute {
        let dispute = self.open_dispute(dispute_id);
        let round = current_round(&dispute).clone();
        require!(!round.tallied, "This round is already counted");
        let now = env::block_timestamp();
        require!(now >= round.reveal_ends.0, format!("Revealing closes at {} ns", round.reveal_ends.0));
        // The largest jury cannot be appealed, so its window closes at once.
        let window = if self.can_appeal(&round) { self.config.appeal_period_ns.0 } else { 0 };
        let counted = Round {
            tallied: true,
            outcome: plurality(&round.votes, dispute.outcomes.len()),
            appeal_ends: Some(U64(now + window)),
            ..round
        };
        self.save_round(dispute_id.0, &dispute, counted);
        self.dispute(dispute_id).unwrap()
    }

    /// Asks a jury of twice the size plus one to decide again, backing `outcome`. The deposit,
    /// less the new round's storage, is that jury's fee; it is repaid if `outcome` is the final ruling.
    #[payable]
    pub fn appeal(&mut self, dispute_id: U64, outcome: u8) -> Dispute {
        let dispute = self.open_dispute(dispute_id);
        let round = current_round(&dispute).clone();
        require!(round.tallied, NOT_COUNTED);
        require!(
            self.can_appeal(&round),
            format!("No more appeals: {} jurors is the largest jury", round.jurors.len())
        );
        let appeal_ends = round.appeal_ends.map_or(0, |ends| ends.0);
        require!(env::block_timestamp() < appeal_ends, format!("Appeals closed at {appeal_ends} ns"));
        require!((outcome as usize) < dispute.outcomes.len(), "There is no such outcome");
        require!(round.outcome != Some(outcome), "Appeal for an outcome other than the round's");

        let deposit = env::attached_deposit();
        let size = round.jurors.len() as u32 * 2 + 1;
        // Nobody sits on two juries of the same dispute, so an appeal really asks new people.
        let earlier: Vec<AccountId> = dispute.rounds.iter().flat_map(|r| r.jurors.clone()).collect();
        let next = self.draw_round(dispute_id.0, dispute.rounds.len(), size, deposit, &earlier);
        let appeal = Appeal {
            by: env::predecessor_account_id(),
            outcome,
            paid: deposit,
        };
        let drawn = with_last_rounds(&dispute, round.clone(), Some(appeal.clone()), next.clone());
        let storage = self.store_dispute(dispute_id.0, &drawn);
        let fee = deposit.saturating_sub(storage);
        let needed = self.config.appeal_fee_per_juror.saturating_mul(size as u128);
        require!(fee >= needed, format!("Attach at least {needed} plus {storage} for storage"));
        let appealed = with_last_rounds(&dispute, round, Some(Appeal { paid: fee, ..appeal }), Round { fee, ..next });
        self.disputes.insert(dispute_id.0, appealed.clone());
        appealed
    }

    /// Makes the last round's outcome the ruling once it can no longer be appealed, pays every
    /// round's jurors against it, releases their locks and sends the ruling to the arbitrable.
    pub fn finalize(&mut self, dispute_id: U64) -> Promise {
        let dispute = self.open_dispute(dispute_id);
        let last = current_round(&dispute).clone();
        require!(last.tallied, NOT_COUNTED);
        let appeal_ends = last.appeal_ends.map_or(0, |ends| ends.0);
        require!(env::block_timestamp() >= appeal_ends, format!("Appeals close at {appeal_ends} ns"));
        let ruling = last.outcome;
        let leftover = dispute.rounds.iter().map(|round| self.pay_round(round, ruling)).sum::<u128>();
        self.pay_leftover(&last, ruling, leftover, &dispute.arbitrable);
        let finalized = Dispute {
            finalized: true,
            ruling,
            ..dispute
        };
        self.disputes.insert(dispute_id.0, finalized.clone());
        send_ruling(dispute_id, finalized.arbitrable, ruling)
    }

    /// Sends a final ruling again when the last delivery failed.
    pub fn deliver_ruling(&mut self, dispute_id: U64) -> Promise {
        let dispute = self.dispute(dispute_id).unwrap_or_else(|| env::panic_str("There is no such dispute"));
        require!(dispute.finalized, "The ruling is not final yet");
        require!(!dispute.ruling_delivered, "The ruling was already delivered");
        send_ruling(dispute_id, dispute.arbitrable, dispute.ruling)
    }

    #[private]
    pub fn on_ruling_delivered(&mut self, dispute_id: U64, #[callback_result] result: Result<(), PromiseError>) -> bool {
        let delivered = result.is_ok();
        if delivered {
            let dispute = self.dispute(dispute_id).unwrap();
            self.disputes.insert(dispute_id.0, Dispute { ruling_delivered: true, ..dispute });
        }
        delivered
    }
}

impl Court {
    fn can_appeal(&self, round: &Round) -> bool {
        // The next jury, 2n + 1, must still fit within max_jury_size.
        (round.jurors.len() as u32) * 2 < self.config.max_jury_size
    }

    /// Pays one round against the final ruling. Jurors who voted otherwise or never revealed
    /// lose their lock. The round's fee and those locks first repay an appellant who was right,
    /// then go to the round's jurors who voted with the ruling. Returns what nobody in the
    /// round could receive.
    fn pay_round(&mut self, round: &Round, ruling: Option<u8>) -> u128 {
        let coherent: Vec<bool> = round.votes.iter().map(|vote| is_coherent(*vote, ruling)).collect();
        let lock = round.lock.as_yoctonear();
        let slashed = coherent.iter().filter(|c| !**c).count() as u128;
        let pot = round.fee.as_yoctonear() + lock * slashed;
        let refund = match &round.appeal {
            Some(appeal) if ruling == Some(appeal.outcome) => appeal.paid.as_yoctonear().min(pot),
            _ => 0,
        };
        if let Some(appeal) = round.appeal.as_ref().filter(|_| refund > 0) {
            Promise::new(appeal.by.clone()).transfer(NearToken::from_yoctonear(refund)).detach();
        }
        let rewards = split(pot - refund, &coherent);
        for ((id, reward), kept) in round.jurors.iter().zip(&rewards).zip(&coherent) {
            self.apply_to_juror(id, *reward, if *kept { 0 } else { lock }, lock);
        }
        if coherent.contains(&true) {
            0
        } else {
            pot - refund
        }
    }

    /// Gives what no round could pay out to the last round's jurors who voted with the ruling,
    /// or back to the arbitrable when there are none.
    fn pay_leftover(&mut self, last: &Round, ruling: Option<u8>, leftover: u128, arbitrable: &AccountId) {
        if leftover == 0 {
            return;
        }
        let coherent: Vec<bool> = last.votes.iter().map(|vote| is_coherent(*vote, ruling)).collect();
        if !coherent.contains(&true) {
            Promise::new(arbitrable.clone()).transfer(NearToken::from_yoctonear(leftover)).detach();
            return;
        }
        // Only jurors who matched the ruling; one who lost their whole stake may already be gone.
        let rewards = split(leftover, &coherent);
        for ((id, reward), _) in last.jurors.iter().zip(rewards).zip(&coherent).filter(|(_, kept)| **kept) {
            self.apply_to_juror(id, reward, 0, 0);
        }
    }

    fn apply_to_juror(&mut self, id: &AccountId, reward: u128, loss: u128, unlock: u128) {
        let juror = self.jurors.get(id).cloned().unwrap();
        let next = Juror {
            staked: NearToken::from_yoctonear(juror.staked.as_yoctonear() + reward - loss),
            locked: NearToken::from_yoctonear(juror.locked.as_yoctonear() - unlock),
            ..juror
        };
        if next.staked.is_zero() && next.unstaking.is_none() {
            self.jurors.remove(id);
        } else {
            self.jurors.insert(id.clone(), next);
        }
    }
}

fn send_ruling(dispute_id: U64, arbitrable: AccountId, ruling: Option<u8>) -> Promise {
    ext_arbitrable::ext(arbitrable)
        .with_static_gas(ON_RULING_GAS)
        .on_ruling(dispute_id, ruling)
        .then(
            Court::ext(env::current_account_id())
                .with_static_gas(ON_RULING_DELIVERED_GAS)
                .on_ruling_delivered(dispute_id),
        )
}

/// The dispute with its last round replaced by `appealed` (carrying `appeal`) and `next` added.
fn with_last_rounds(dispute: &Dispute, appealed: Round, appeal: Option<Appeal>, next: Round) -> Dispute {
    let rounds = [&dispute.rounds[..dispute.rounds.len() - 1], &[Round { appeal, ..appealed }, next]].concat();
    Dispute { rounds, ..dispute.clone() }
}

/// With a ruling, a vote is coherent when it chose it. With no ruling (a final tie), every
/// revealed vote is; only missed reveals lose their lock.
fn is_coherent(vote: Option<u8>, ruling: Option<u8>) -> bool {
    match ruling {
        Some(outcome) => vote == Some(outcome),
        None => vote.is_some(),
    }
}

/// `amount` shared equally by the marked seats; the first marked seat also gets the remainder.
fn split(amount: u128, marked: &[bool]) -> Vec<u128> {
    let count = marked.iter().filter(|m| **m).count() as u128;
    let share = amount.checked_div(count).unwrap_or(0);
    let first = marked.iter().position(|m| *m);
    marked
        .iter()
        .enumerate()
        .map(|(seat, m)| match (*m, Some(seat) == first) {
            (true, true) => share + (amount - share * count),
            (true, false) => share,
            _ => 0,
        })
        .collect()
}

/// The outcome with the most revealed votes, or None when nobody revealed or the top is tied.
fn plurality(votes: &[Option<u8>], outcomes: usize) -> Option<u8> {
    let counts: Vec<usize> = (0..outcomes).map(|o| votes.iter().filter(|v| **v == Some(o as u8)).count()).collect();
    let top = *counts.iter().max()?;
    let leaders: Vec<usize> = (0..outcomes).filter(|o| counts[*o] == top).collect();
    (top > 0 && leaders.len() == 1).then(|| leaders[0] as u8)
}
