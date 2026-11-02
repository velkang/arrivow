use near_sdk::json_types::U64;
use near_sdk::{env, near, require, AccountId, NearToken, Promise};

use crate::{Court, CourtExt, MAX_JURORS};

#[near(serializers = [json, borsh])]
#[derive(Clone, Debug, PartialEq)]
pub struct Unstaking {
    pub amount: NearToken,
    /// Block time from which the amount can be withdrawn, in nanoseconds.
    pub ready_at: U64,
}

#[near(serializers = [json, borsh])]
#[derive(Clone, Debug, PartialEq)]
pub struct Juror {
    pub staked: NearToken,
    /// Stake held by open disputes; it cannot be asked back.
    pub locked: NearToken,
    pub unstaking: Option<Unstaking>,
}

impl Juror {
    /// Stake that can still be drawn into a dispute or asked back.
    pub fn available(&self) -> NearToken {
        let asked_back = self.unstaking.as_ref().map_or(NearToken::from_yoctonear(0), |u| u.amount);
        self.staked.saturating_sub(self.locked).saturating_sub(asked_back)
    }
}

#[near]
impl Court {
    /// Adds the attached deposit to the caller's stake.
    #[payable]
    pub fn stake(&mut self) -> Juror {
        let account_id = env::predecessor_account_id();
        require!(
            self.jurors.contains_key(&account_id) || self.jurors.len() < MAX_JURORS,
            format!("The court is full at {MAX_JURORS} jurors")
        );
        let current = self.jurors.get(&account_id).cloned().unwrap_or(Juror {
            staked: NearToken::from_yoctonear(0),
            locked: NearToken::from_yoctonear(0),
            unstaking: None,
        });
        let juror = Juror {
            staked: current.staked.saturating_add(env::attached_deposit()),
            ..current
        };
        require!(
            juror.staked >= self.config.min_stake,
            format!("Stake at least {}", self.config.min_stake)
        );
        self.jurors.insert(account_id, juror.clone());
        juror
    }

    /// Takes `amount` out of the draw; it can be withdrawn after the unstake delay.
    pub fn request_unstake(&mut self, amount: NearToken) -> Juror {
        let account_id = env::predecessor_account_id();
        let current = self.jurors.get(&account_id).cloned().unwrap_or_else(|| env::panic_str("This account has no stake"));
        require!(current.unstaking.is_none(), "Withdraw the stake already asked back first");
        require!(amount.as_yoctonear() > 0, "Ask back more than 0 NEAR");
        require!(amount <= current.available(), format!("Only {} is available", current.available()));
        let remaining = current.staked.saturating_sub(amount);
        require!(
            remaining.is_zero() || remaining >= self.config.min_stake,
            format!("Leave at least {} staked or unstake everything", self.config.min_stake)
        );
        let juror = Juror {
            unstaking: Some(Unstaking {
                amount,
                ready_at: U64(env::block_timestamp() + self.config.unstake_delay_ns.0),
            }),
            ..current
        };
        self.jurors.insert(account_id, juror.clone());
        juror
    }

    /// Pays out stake asked back once its delay has passed.
    pub fn withdraw(&mut self) -> Promise {
        let account_id = env::predecessor_account_id();
        let current = self.jurors.get(&account_id).cloned().unwrap_or_else(|| env::panic_str("This account has no stake"));
        let unstaking = current.unstaking.clone().unwrap_or_else(|| env::panic_str("Nothing has been asked back"));
        require!(
            env::block_timestamp() >= unstaking.ready_at.0,
            format!("Stake asked back can be withdrawn from {} ns", unstaking.ready_at.0)
        );
        let juror = Juror {
            staked: current.staked.saturating_sub(unstaking.amount),
            unstaking: None,
            ..current
        };
        if juror.staked.is_zero() {
            self.jurors.remove(&account_id);
        } else {
            self.jurors.insert(account_id.clone(), juror);
        }
        // State changes before the transfer; a failed transfer refunds the court, not the juror.
        Promise::new(account_id).transfer(unstaking.amount)
    }

    pub fn juror(&self, account_id: AccountId) -> Option<Juror> {
        self.jurors.get(&account_id).cloned()
    }

    pub fn jurors(&self, from_index: Option<u32>, limit: Option<u32>) -> Vec<(AccountId, Juror)> {
        self.jurors
            .iter()
            .skip(from_index.unwrap_or(0) as usize)
            .take(limit.unwrap_or(50).min(100) as usize)
            .map(|(id, juror)| (id.clone(), juror.clone()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::{account, call_as, court, near, zero, DELAY_NS};
    use near_sdk::test_utils::get_created_receipts;

    fn staked_court(name: &str, amount: u128) -> Court {
        let mut court = court();
        call_as(name, near(amount), 0);
        court.stake();
        court
    }

    #[test]
    fn stake_adds_the_deposit_to_the_callers_stake() {
        let mut court = staked_court("alice.testnet", 2);
        call_as("alice.testnet", near(3), 0);

        let juror = court.stake();

        assert_eq!(juror.staked, near(5));
        assert_eq!(juror.available(), near(5));
        assert_eq!(court.juror(account("alice.testnet")), Some(juror));
    }

    #[test]
    #[should_panic(expected = "Stake at least 1.00 NEAR")]
    fn stake_refuses_a_first_stake_below_the_minimum() {
        let mut court = court();
        call_as("alice.testnet", NearToken::from_millinear(500), 0);

        court.stake();
    }

    #[test]
    fn stake_accepts_a_small_top_up_once_the_minimum_is_held() {
        let mut court = staked_court("alice.testnet", 1);
        call_as("alice.testnet", NearToken::from_millinear(100), 0);

        assert_eq!(court.stake().staked, NearToken::from_millinear(1_100));
    }

    #[test]
    fn request_unstake_takes_the_amount_out_of_the_draw() {
        let mut court = staked_court("alice.testnet", 5);
        call_as("alice.testnet", zero(), 10);

        let juror = court.request_unstake(near(2));

        assert_eq!(juror.staked, near(5));
        assert_eq!(juror.available(), near(3));
        assert_eq!(
            juror.unstaking,
            Some(Unstaking {
                amount: near(2),
                ready_at: U64(10 + DELAY_NS),
            })
        );
    }

    #[test]
    #[should_panic(expected = "Only 5.00 NEAR is available")]
    fn request_unstake_refuses_more_than_is_available() {
        let mut court = staked_court("alice.testnet", 5);
        call_as("alice.testnet", zero(), 0);

        court.request_unstake(near(6));
    }

    #[test]
    #[should_panic(expected = "Leave at least 1.00 NEAR staked or unstake everything")]
    fn request_unstake_refuses_to_leave_less_than_the_minimum() {
        let mut court = staked_court("alice.testnet", 5);
        call_as("alice.testnet", zero(), 0);

        court.request_unstake(NearToken::from_millinear(4_500));
    }

    #[test]
    #[should_panic(expected = "Withdraw the stake already asked back first")]
    fn request_unstake_refuses_a_second_request_while_one_waits() {
        let mut court = staked_court("alice.testnet", 5);
        call_as("alice.testnet", zero(), 0);
        court.request_unstake(near(1));

        court.request_unstake(near(1));
    }

    #[test]
    #[should_panic(expected = "This account has no stake")]
    fn request_unstake_refuses_an_account_with_no_stake() {
        let mut court = court();
        call_as("bob.testnet", zero(), 0);

        court.request_unstake(near(1));
    }

    #[test]
    #[should_panic(expected = "Stake asked back can be withdrawn from")]
    fn withdraw_refuses_before_the_delay_has_passed() {
        let mut court = staked_court("alice.testnet", 5);
        call_as("alice.testnet", zero(), 0);
        court.request_unstake(near(2));
        call_as("alice.testnet", zero(), DELAY_NS - 1);

        let _ = court.withdraw();
    }

    #[test]
    #[should_panic(expected = "Nothing has been asked back")]
    fn withdraw_refuses_when_nothing_was_asked_back() {
        let mut court = staked_court("alice.testnet", 5);
        call_as("alice.testnet", zero(), 0);

        let _ = court.withdraw();
    }

    #[test]
    fn withdraw_pays_the_amount_and_lowers_the_stake() {
        let mut court = staked_court("alice.testnet", 5);
        call_as("alice.testnet", zero(), 0);
        court.request_unstake(near(2));
        call_as("alice.testnet", zero(), DELAY_NS);

        let _ = court.withdraw();

        let juror = court.juror(account("alice.testnet")).unwrap();
        assert_eq!((juror.staked, juror.unstaking), (near(3), None));
        let receipts = get_created_receipts();
        assert_eq!(receipts.len(), 1);
        assert_eq!(receipts[0].receiver_id, account("alice.testnet"));
        assert!(format!("{:?}", receipts[0].actions).contains(&near(2).as_yoctonear().to_string()));
    }

    #[test]
    fn withdrawing_everything_removes_the_juror() {
        let mut court = staked_court("alice.testnet", 5);
        call_as("alice.testnet", zero(), 0);
        court.request_unstake(near(5));
        call_as("alice.testnet", zero(), DELAY_NS);

        let _ = court.withdraw();

        assert_eq!(court.juror(account("alice.testnet")), None);
        assert!(court.jurors(None, None).is_empty());
    }

    #[test]
    #[should_panic(expected = "The court is full at 200 jurors")]
    fn stake_refuses_a_new_juror_once_the_court_is_full() {
        let mut court = court();
        for index in 0..=crate::MAX_JURORS {
            call_as(&format!("juror{index}.testnet"), near(1), 0);
            court.stake();
        }
    }

    #[test]
    fn jurors_lists_a_page_of_jurors() {
        let mut court = staked_court("alice.testnet", 1);
        for name in ["bob.testnet", "carol.testnet"] {
            call_as(name, near(2), 0);
            court.stake();
        }

        let page: Vec<AccountId> = court.jurors(Some(1), Some(5)).into_iter().map(|(id, _)| id).collect();

        assert_eq!(page, vec![account("bob.testnet"), account("carol.testnet")]);
    }
}
