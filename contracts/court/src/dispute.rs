use near_sdk::json_types::{Base64VecU8, U64};
use near_sdk::{env, near, require, AccountId, NearToken};

use crate::{Court, CourtExt, Juror};

#[cfg(test)]
mod tests;

#[near(serializers = [json, borsh])]
#[derive(Clone, Debug, PartialEq)]
pub struct Appeal {
    pub by: AccountId,
    /// The outcome the appellant says is right; it differs from the round's outcome.
    pub outcome: u8,
    /// What the appellant paid the next jury; repaid if the final ruling is `outcome`.
    pub paid: NearToken,
}

#[near(serializers = [json, borsh])]
#[derive(Clone, Debug, PartialEq)]
pub struct Round {
    pub jurors: Vec<AccountId>,
    /// Stake locked from each drawn juror until the dispute's ruling is final.
    pub lock: NearToken,
    /// Paid to this round's jurors who vote with the final ruling.
    pub fee: NearToken,
    pub commit_ends: U64,
    pub reveal_ends: U64,
    /// `commitment_hash` of each juror's vote, by position in `jurors`.
    pub commitments: Vec<Option<Base64VecU8>>,
    /// Revealed outcomes, by position in `jurors`.
    pub votes: Vec<Option<u8>>,
    pub tallied: bool,
    /// The outcome most revealed votes chose; None until tallied, and after a tie.
    pub outcome: Option<u8>,
    /// Set when the round is tallied; the outcome can be appealed until then.
    pub appeal_ends: Option<U64>,
    pub appeal: Option<Appeal>,
}

#[near(serializers = [json, borsh])]
#[derive(Clone, Debug, PartialEq)]
pub struct Dispute {
    /// The account that opened the dispute; it receives the ruling through `on_ruling`.
    pub arbitrable: AccountId,
    pub question: String,
    pub outcomes: Vec<String>,
    pub rounds: Vec<Round>,
    pub finalized: bool,
    /// The final ruling; None while open, and when the last round ended in a tie.
    pub ruling: Option<u8>,
    /// Whether `on_ruling` succeeded on the arbitrable account.
    pub ruling_delivered: bool,
}

/// sha256(outcome || salt || juror account id). Binding the juror's account stops a
/// juror from copying another's commitment and revealing their vote once it is public.
pub fn commitment_hash(outcome: u8, salt: &[u8], juror: &AccountId) -> Vec<u8> {
    env::sha256([&[outcome], salt, juror.as_bytes()].concat())
}

const MAX_OUTCOMES: usize = 8;
const MAX_QUESTION_BYTES: usize = 500;
pub(crate) const TALLIED_ROUND: &str = "Voting for this round is over";

#[near]
impl Court {
    /// Opens a dispute for the caller and draws its first jury. The attached deposit, less the
    /// dispute's storage, is the first round's fee.
    #[payable]
    pub fn create_dispute(&mut self, question: String, outcomes: Vec<String>) -> U64 {
        require!(
            (2..=MAX_OUTCOMES).contains(&outcomes.len()),
            format!("A dispute needs 2 to {MAX_OUTCOMES} outcomes")
        );
        require!(
            !question.is_empty() && question.len() <= MAX_QUESTION_BYTES,
            format!("A question is 1 to {MAX_QUESTION_BYTES} bytes")
        );
        let deposit = env::attached_deposit();
        let id = self.next_dispute_id;
        let round = self.draw_round(id, 0, self.config.first_round_jurors, deposit, &[]);
        let dispute = Dispute {
            arbitrable: env::predecessor_account_id(),
            question,
            outcomes,
            rounds: vec![round],
            finalized: false,
            ruling: None,
            ruling_delivered: false,
        };
        let storage = self.store_dispute(id, &dispute);
        let fee = deposit.saturating_sub(storage);
        require!(
            fee >= self.config.min_fee,
            format!("Attach a fee of at least {} plus {} for storage", self.config.min_fee, storage)
        );
        self.save_round(id, &dispute, Round { fee, ..current_round(&dispute).clone() });
        self.next_dispute_id = id + 1;
        U64(id)
    }

    /// Records a drawn juror's hidden vote; it can be replaced until voting closes.
    pub fn commit(&mut self, dispute_id: U64, commitment: Base64VecU8) {
        require!(commitment.0.len() == 32, "A commitment is a 32-byte sha256 hash");
        let dispute = self.open_dispute(dispute_id);
        let round = current_round(&dispute);
        let seat = seat_of(round, &env::predecessor_account_id());
        require!(!round.tallied, TALLIED_ROUND);
        require!(env::block_timestamp() < round.commit_ends.0, "Voting has closed");
        let next = Round {
            commitments: replaced(&round.commitments, seat, Some(commitment)),
            ..round.clone()
        };
        self.save_round(dispute_id.0, &dispute, next);
    }

    /// Opens a juror's vote once voting has closed.
    pub fn reveal(&mut self, dispute_id: U64, outcome: u8, salt: Base64VecU8) {
        let juror = env::predecessor_account_id();
        let dispute = self.open_dispute(dispute_id);
        let round = current_round(&dispute);
        let seat = seat_of(round, &juror);
        require!(!round.tallied, TALLIED_ROUND);
        let now = env::block_timestamp();
        require!(now >= round.commit_ends.0, "Votes can be revealed once voting closes");
        require!(now < round.reveal_ends.0, "Revealing has closed");
        require!((outcome as usize) < dispute.outcomes.len(), "There is no such outcome");
        require!(round.votes[seat].is_none(), "This vote is already revealed");
        let committed = round.commitments[seat].as_ref().unwrap_or_else(|| env::panic_str("You did not commit a vote"));
        require!(
            committed.0 == commitment_hash(outcome, &salt.0, &juror),
            "This does not match your commitment"
        );
        let next = Round {
            votes: replaced(&round.votes, seat, Some(outcome)),
            ..round.clone()
        };
        self.save_round(dispute_id.0, &dispute, next);
    }

    pub fn dispute(&self, dispute_id: U64) -> Option<Dispute> {
        self.disputes.get(&dispute_id.0).cloned()
    }
}

impl Court {
    pub(crate) fn open_dispute(&self, dispute_id: U64) -> Dispute {
        let dispute = self.dispute(dispute_id).unwrap_or_else(|| env::panic_str("There is no such dispute"));
        require!(!dispute.finalized, "This dispute has a final ruling");
        dispute
    }

    /// Replaces the dispute's last round.
    pub(crate) fn save_round(&mut self, dispute_id: u64, dispute: &Dispute, round: Round) {
        let rounds = [&dispute.rounds[..dispute.rounds.len() - 1], &[round]].concat();
        self.disputes.insert(dispute_id, Dispute { rounds, ..dispute.clone() });
    }

    /// Writes the dispute and returns what its added storage costs.
    pub(crate) fn store_dispute(&mut self, dispute_id: u64, dispute: &Dispute) -> NearToken {
        let before = env::storage_usage();
        self.disputes.insert(dispute_id, dispute.clone());
        self.disputes.flush();
        let added = env::storage_usage().saturating_sub(before);
        env::storage_byte_cost().saturating_mul(added as u128)
    }

    /// Draws `size` distinct jurors outside `excluded`, each with chance proportional to their
    /// available stake, and locks `juror_lock` of each one's stake.
    pub(crate) fn draw_round(
        &mut self,
        dispute_id: u64,
        round_index: usize,
        size: u32,
        fee: NearToken,
        excluded: &[AccountId],
    ) -> Round {
        let lock = self.config.juror_lock;
        let candidates: Vec<(AccountId, u128)> = self
            .jurors
            .iter()
            .filter(|(id, juror)| juror.available() >= lock && !excluded.contains(id))
            .map(|(id, juror)| (id.clone(), juror.available().as_yoctonear()))
            .collect();
        require!(
            candidates.len() >= size as usize,
            format!("Only {} jurors can be drawn; {size} are needed", candidates.len())
        );
        let seed = env::random_seed();
        let (jurors, _) = (0..size).fold((Vec::new(), candidates), |(drawn, pool), draw| {
            let total: u128 = pool.iter().map(|(_, weight)| weight).sum();
            let index = weighted_index(&pool, random_below(&seed, dispute_id, round_index, draw, total));
            let rest = [&pool[..index], &pool[index + 1..]].concat();
            ([drawn, vec![pool[index].0.clone()]].concat(), rest)
        });
        for id in &jurors {
            let juror = self.jurors.get(id).cloned().unwrap();
            self.jurors.insert(id.clone(), Juror { locked: juror.locked.saturating_add(lock), ..juror });
        }
        let commit_ends = env::block_timestamp() + self.config.commit_period_ns.0;
        Round {
            commitments: vec![None; jurors.len()],
            votes: vec![None; jurors.len()],
            jurors,
            lock,
            fee,
            commit_ends: U64(commit_ends),
            reveal_ends: U64(commit_ends + self.config.reveal_period_ns.0),
            tallied: false,
            outcome: None,
            appeal_ends: None,
            appeal: None,
        }
    }
}

pub(crate) fn current_round(dispute: &Dispute) -> &Round {
    dispute.rounds.last().expect("a dispute always has a round")
}

fn seat_of(round: &Round, juror: &AccountId) -> usize {
    round
        .jurors
        .iter()
        .position(|id| id == juror)
        .unwrap_or_else(|| env::panic_str("Only jurors drawn for this round can vote"))
}

fn replaced<T: Clone>(items: &[T], index: usize, value: T) -> Vec<T> {
    items.iter().enumerate().map(|(i, item)| if i == index { value.clone() } else { item.clone() }).collect()
}

/// A number below `total` from the block's random seed, distinct for each draw.
fn random_below(seed: &[u8], dispute_id: u64, round_index: usize, draw: u32, total: u128) -> u128 {
    let input = [seed, &dispute_id.to_le_bytes(), &(round_index as u32).to_le_bytes(), &draw.to_le_bytes()].concat();
    let hash = env::sha256(&input);
    u128::from_le_bytes(hash[..16].try_into().unwrap()) % total
}

/// The position whose cumulative weight first exceeds `pick`.
fn weighted_index(pool: &[(AccountId, u128)], pick: u128) -> usize {
    pool.iter()
        .scan(0u128, |sum, (_, weight)| {
            *sum += weight;
            Some(*sum)
        })
        .position(|cumulative| cumulative > pick)
        .expect("pick is below the total weight")
}
