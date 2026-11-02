use near_sdk::json_types::U64;
use near_sdk::{env, ext_contract, near, require, Gas, Promise, PromiseError};

use crate::{pay, Market, Markets, MarketsExt, Status, OUTCOMES};

#[cfg(test)]
mod tests;

/// The court reads every juror to draw a jury, so it gets most of the gas.
const CREATE_DISPUTE_GAS: Gas = Gas::from_tgas(100);
/// The callback only writes the market; a wide margin, since a callback that runs out of gas
/// would leave the market in Challenging with no way out.
const ON_DISPUTE_CREATED_GAS: Gas = Gas::from_tgas(20);
const MIN_CHALLENGE_GAS: Gas = Gas::from_tgas(150);

/// The part of the court contract the markets use.
#[ext_contract(ext_court)]
pub trait Court {
    fn create_dispute(&mut self, question: String, outcomes: Vec<String>) -> U64;
}

#[near]
impl Markets {
    /// Proposes the outcome of a closed market, putting up the proposal bond.
    #[payable]
    pub fn propose(&mut self, market_id: U64, outcome: u8) -> Market {
        let market = self.market_or_panic(market_id);
        require!(market.status == Status::Open, "This market is not waiting for a proposal");
        let now = env::block_timestamp();
        require!(now >= market.closes_at.0, "Outcomes can be proposed once betting closes");
        require!((outcome as usize) < OUTCOMES, "Propose 0 (Yes) or 1 (No)");
        let bond = self.config.proposal_bond;
        require!(env::attached_deposit() == bond, format!("Attach exactly the {bond} bond"));
        let proposed = Market {
            status: Status::Proposed {
                outcome,
                proposer: env::predecessor_account_id(),
                challenge_ends: U64(now + self.config.challenge_period_ns.0),
            },
            ..market
        };
        self.save_market(market_id, proposed.clone());
        proposed
    }

    /// Makes an unchallenged proposal the outcome and returns the proposer's bond.
    pub fn finalize_proposal(&mut self, market_id: U64) -> Market {
        let market = self.market_or_panic(market_id);
        let Status::Proposed { outcome, proposer, challenge_ends } = market.status.clone() else {
            env::panic_str("This market has no proposal to finalize")
        };
        require!(
            env::block_timestamp() >= challenge_ends.0,
            format!("Challenges close at {} ns", challenge_ends.0)
        );
        pay(&proposer, self.config.proposal_bond);
        let resolved = Market {
            status: Status::Resolved { outcome: Some(outcome) },
            ..market
        };
        self.save_market(market_id, resolved.clone());
        resolved
    }

    /// Challenges the proposed outcome with an equal bond plus the court fee, which opens a
    /// dispute in the court.
    #[payable]
    pub fn challenge(&mut self, market_id: U64) -> Promise {
        let market = self.market_or_panic(market_id);
        let Status::Proposed { outcome, proposer, challenge_ends } = market.status.clone() else {
            env::panic_str("This market has no proposal to challenge")
        };
        require!(
            env::block_timestamp() < challenge_ends.0,
            format!("Challenges closed at {} ns", challenge_ends.0)
        );
        let (bond, fee) = (self.config.proposal_bond, self.config.court_fee);
        require!(
            env::attached_deposit() == bond.saturating_add(fee),
            format!("Attach the {bond} bond plus the {fee} court fee")
        );
        require!(env::prepaid_gas() >= MIN_CHALLENGE_GAS, "Attach at least 150 Tgas so the court can draw a jury");
        let question = market.question.clone();
        self.save_market(
            market_id,
            Market {
                status: Status::Challenging {
                    outcome,
                    proposer,
                    challenge_ends,
                    challenger: env::predecessor_account_id(),
                },
                ..market
            },
        );
        ext_court::ext(self.config.court.clone())
            .with_attached_deposit(fee)
            .with_static_gas(CREATE_DISPUTE_GAS)
            .create_dispute(question, vec!["Yes".into(), "No".into()])
            .then(
                Self::ext(env::current_account_id())
                    .with_static_gas(ON_DISPUTE_CREATED_GAS)
                    .on_dispute_created(market_id),
            )
    }

    #[private]
    pub fn on_dispute_created(&mut self, market_id: U64, #[callback_result] result: Result<U64, PromiseError>) -> bool {
        let market = self.market_or_panic(market_id);
        let Status::Challenging { outcome, proposer, challenge_ends, challenger } = market.status.clone() else {
            return false;
        };
        let (status, created) = match result {
            Ok(dispute_id) => {
                self.disputed.insert(dispute_id.0, market_id.0);
                (Status::Disputed { outcome, proposer, challenger, dispute_id }, true)
            }
            Err(_) => {
                // The court refunded its fee to this contract; both go back to the challenger.
                pay(&challenger, self.config.proposal_bond.saturating_add(self.config.court_fee));
                (Status::Proposed { outcome, proposer, challenge_ends }, false)
            }
        };
        self.save_market(market_id, Market { status, ..market });
        created
    }

    /// The court's final ruling. The side the ruling backs takes both bonds; with no ruling each
    /// side gets its own back. Repeats are ignored.
    pub fn on_ruling(&mut self, dispute_id: U64, ruling: Option<u8>) {
        require!(env::predecessor_account_id() == self.config.court, "Only the court can rule");
        // Removed once ruled, which frees its storage and makes any repeat a no-op.
        let Some(market_id) = self.disputed.remove(&dispute_id.0) else {
            return;
        };
        let market = self.market_or_panic(U64(market_id));
        let Status::Disputed { outcome, proposer, challenger, .. } = market.status.clone() else {
            return;
        };
        let bond = self.config.proposal_bond;
        match ruling {
            Some(ruled) if ruled == outcome => pay(&proposer, bond.saturating_mul(2)),
            Some(_) => pay(&challenger, bond.saturating_mul(2)),
            None => {
                pay(&proposer, bond);
                pay(&challenger, bond);
            }
        }
        self.save_market(
            U64(market_id),
            Market {
                status: Status::Resolved { outcome: ruling },
                ..market
            },
        );
    }
}
