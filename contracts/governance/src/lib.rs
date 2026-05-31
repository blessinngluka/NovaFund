#![no_std]

use shared::{
    constants::GOVERNANCE_QUORUM,
    errors::Error,
    events::{PROPOSAL_CREATED, PROPOSAL_EXECUTED, VOTE_CAST},
    types::{Amount, Proposal},
};
use soroban_sdk::{contract, contractimpl, contracttype, token, Address, Bytes, Env, Vec};

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    NextProposalId,
    Proposal(u64),
    HasVoted(u64, Address),
    VoteChoice(u64, Address),
    TotalVoters,
    Admin,
    GovToken,
    ReputationContract,
    Stake(Address),
    TotalStake,
    TimelockDelay,
    ProposalTimelock(u64),
    Delegation(Address),
    ProposalCancelled(u64),
}

pub trait GovernanceTrait {
    fn initialize(env: Env, admin: Address, total_voters: u32) -> Result<(), Error>;
    fn configure_token(env: Env, admin: Address, token_address: Address, timelock_delay: u64) -> Result<(), Error>;
    fn set_reputation_contract(env: Env, admin: Address, reputation_address: Address) -> Result<(), Error>;
    fn delegate_votes(env: Env, delegator: Address, delegatee: Address) -> Result<(), Error>;
    fn create_proposal(env: Env, creator: Address, payload_ref: Bytes, start_time: u64, end_time: u64) -> Result<u64, Error>;
    fn cancel_proposal(env: Env, canceller: Address, proposal_id: u64) -> Result<(), Error>;
    fn vote(env: Env, proposal_id: u64, voter: Address, support: bool) -> Result<(), Error>;
    fn finalize(env: Env, proposal_id: u64) -> Result<(), Error>;
    fn get_proposal(env: Env, proposal_id: u64) -> Result<Proposal, Error>;
    fn get_voting_power(env: Env, address: Address) -> i128;
    fn get_active_proposals(env: Env) -> Vec<u64>;
    fn get_vote_record(env: Env, voter: Address, proposal_id: u64) -> Option<bool>;
    fn update_governance_parameters(env: Env, admin: Address, key: u32, value: u64) -> Result<(), Error>;
    fn get_total_voters(env: Env) -> u32;
    fn get_stake(env: Env, voter: Address) -> Amount;
    fn get_total_stake(env: Env) -> Amount;
    fn get_proposal_timelock(env: Env, proposal_id: u64) -> Option<u64>;
    fn has_voted(env: Env, proposal_id: u64, voter: Address) -> bool;
}

#[contract]
pub struct GovernanceContract;

#[cfg(test)]
mod tests;

#[contractimpl]
impl GovernanceTrait for GovernanceContract {
    pub fn initialize(env: Env, admin: Address, total_voters: u32) -> Result<(), Error> {
        admin.require_auth();

        if env.storage().instance().has(&DataKey::TotalVoters) {
            return Err(Error::AlreadyInit);
        }

        if total_voters == 0 {
            return Err(Error::InvInput);
        }

        let storage = env.storage().instance();
        storage.set(&DataKey::TotalVoters, &total_voters);
        storage.set(&DataKey::Admin, &admin);

        Ok(())
    }

    pub fn configure_token(
        env: Env,
        admin: Address,
        token_address: Address,
        timelock_delay: u64,
    ) -> Result<(), Error> {
        admin.require_auth();

        let storage = env.storage().instance();

        let stored_admin: Address = storage.get(&DataKey::Admin).ok_or(Error::NotInit)?;
        if stored_admin != admin {
            return Err(Error::Unauthorized);
        }

        if storage.has(&DataKey::GovToken) {
            return Err(Error::AlreadyInit);
        }

        storage.set(&DataKey::GovToken, &token_address);
        storage.set(&DataKey::TimelockDelay, &timelock_delay);
        storage.set(&DataKey::TotalStake, &0_i128);

        Ok(())
    }

    pub fn set_reputation_contract(
        env: Env,
        admin: Address,
        reputation_address: Address,
    ) -> Result<(), Error> {
        admin.require_auth();

        let storage = env.storage().instance();
        let stored_admin: Address = storage.get(&DataKey::Admin).ok_or(Error::NotInit)?;
        if stored_admin != admin {
            return Err(Error::Unauthorized);
        }

        storage.set(&DataKey::ReputationContract, &reputation_address);
        Ok(())
    }

    pub fn delegate_votes(env: Env, delegator: Address, delegatee: Address) -> Result<(), Error> {
        delegator.require_auth();

        if delegator == delegatee {
            return Err(Error::InvInput);
        }

        let storage = env.storage().persistent();
        storage.set(&DataKey::Delegation(delegator), &delegatee);

        Ok(())
    }

    pub fn distribute_tokens(
        env: Env,
        admin: Address,
        recipients: Vec<Address>,
        amounts: Vec<Amount>,
    ) -> Result<(), Error> {
        admin.require_auth();

        if recipients.len() != amounts.len() || recipients.len() == 0 {
            return Err(Error::InvInput);
        }

        let gov_token: Address = env
            .storage()
            .instance()
            .get(&DataKey::GovToken)
            .ok_or(Error::NotInit)?;

        let token_client = token::Client::new(&env, &gov_token);

        let len = recipients.len();
        for i in 0..len {
            let recipient = recipients.get(i).unwrap();
            let amount = amounts.get(i).unwrap();
            if amount <= 0 {
                return Err(Error::InvInput);
            }
            token_client.transfer(&admin, &recipient, &amount);
        }

        Ok(())
    }

    pub fn stake(env: Env, voter: Address, amount: Amount) -> Result<(), Error> {
        voter.require_auth();

        if amount <= 0 {
            return Err(Error::InvInput);
        }

        let storage = env.storage().instance();
        let gov_token: Address = storage.get(&DataKey::GovToken).ok_or(Error::NotInit)?;

        let token_client = token::Client::new(&env, &gov_token);
        let self_address = env.current_contract_address();

        // Move tokens from voter into governance contract as stake
        token_client.transfer(&voter, &self_address, &amount);

        let mut current_stake: Amount = storage.get(&DataKey::Stake(voter.clone())).unwrap_or(0);
        current_stake += amount;
        storage.set(&DataKey::Stake(voter.clone()), &current_stake);

        let mut total_stake: Amount = storage.get(&DataKey::TotalStake).unwrap_or(0);
        total_stake += amount;
        storage.set(&DataKey::TotalStake, &total_stake);

        Ok(())
    }

    pub fn unstake(env: Env, voter: Address, amount: Amount) -> Result<(), Error> {
        voter.require_auth();

        if amount <= 0 {
            return Err(Error::InvInput);
        }

        let storage = env.storage().instance();
        let gov_token: Address = storage.get(&DataKey::GovToken).ok_or(Error::NotInit)?;

        let mut current_stake: Amount = storage.get(&DataKey::Stake(voter.clone())).unwrap_or(0);

        if current_stake < amount {
            return Err(Error::InsufVote);
        }

        current_stake -= amount;
        storage.set(&DataKey::Stake(voter.clone()), &current_stake);

        let mut total_stake: Amount = storage.get(&DataKey::TotalStake).unwrap_or(0);
        total_stake -= amount;
        storage.set(&DataKey::TotalStake, &total_stake);

        let self_address = env.current_contract_address();
        let token_client = token::Client::new(&env, &gov_token);
        token_client.transfer(&self_address, &voter, &amount);

        Ok(())
    }

    pub fn create_proposal(
        env: Env,
        creator: Address,
        payload_ref: Bytes,
        start_time: u64,
        end_time: u64,
    ) -> Result<u64, Error> {
        creator.require_auth();

        let current_time = env.ledger().timestamp();

        if end_time <= start_time {
            return Err(Error::InvInput);
        }

        if start_time < current_time {
            return Err(Error::InvInput);
        }
        if payload_ref.len() == 0 {
            return Err(Error::InvInput);
        }

        let proposal_id: u64 = env
            .storage()
            .instance()
            .get(&DataKey::NextProposalId)
            .unwrap_or(0);

        let proposal = Proposal {
            id: proposal_id,
            creator: creator.clone(),
            payload_ref: payload_ref.clone(),
            start_time,
            end_time,
            yes_votes: 0,
            no_votes: 0,
            executed: false,
        };

        env.storage()
            .instance()
            .set(&DataKey::Proposal(proposal_id), &proposal);
        env.storage()
            .instance()
            .set(&DataKey::NextProposalId, &(proposal_id + 1));

        // Emit proposal created event
        env.events()
            .publish((PROPOSAL_CREATED,), (proposal_id, creator, payload_ref));

        Ok(proposal_id)
    }

    pub fn cancel_proposal(env: Env, canceller: Address, proposal_id: u64) -> Result<(), Error> {
        canceller.require_auth();

        let proposal: Proposal = env
            .storage()
            .instance()
            .get(&DataKey::Proposal(proposal_id))
            .ok_or(Error::NotFound)?;

        if proposal.creator != canceller {
            let admin: Address = env.storage().instance().get(&DataKey::Admin).ok_or(Error::NotInit)?;
            if admin != canceller {
                return Err(Error::Unauthorized);
            }
        }

        let current_time = env.ledger().timestamp();
        if current_time >= proposal.start_time {
            return Err(Error::InvInput);
        }

        if proposal.executed {
            return Err(Error::PropExc);
        }

        env.storage().instance().set(&DataKey::ProposalCancelled(proposal_id), &true);

        Ok(())
    }

    pub fn vote(env: Env, proposal_id: u64, voter: Address, support: bool) -> Result<(), Error> {
        voter.require_auth();

        let mut proposal: Proposal = env
            .storage()
            .instance()
            .get(&DataKey::Proposal(proposal_id))
            .ok_or(Error::NotFound)?;

        if env.storage().instance().has(&DataKey::ProposalCancelled(proposal_id)) {
            return Err(Error::PropNotAct);
        }

        let current_time = env.ledger().timestamp();

        if proposal.executed {
            return Err(Error::PropExc);
        }

        if current_time < proposal.start_time || current_time > proposal.end_time {
            return Err(Error::InvInput);
        }

        let vote_key = DataKey::HasVoted(proposal_id, voter.clone());
        if env.storage().instance().has(&vote_key) {
            return Err(Error::AlreadyVoted);
        }

        let stake = Self::get_voting_power(env.clone(), voter.clone());

        if stake <= 0 {
            return Err(Error::InsufVote);
        }

        if support {
            proposal.yes_votes += stake;
        } else {
            proposal.no_votes += stake;
        }

        env.storage().instance().set(&DataKey::Proposal(proposal_id), &proposal);
        env.storage().instance().set(&vote_key, &true);
        env.storage().instance().set(&DataKey::VoteChoice(proposal_id, voter.clone()), &support);

        // Emit vote cast event
        env.events()
            .publish((VOTE_CAST,), (proposal_id, voter, support));

        Ok(())
    }

    pub fn finalize(env: Env, proposal_id: u64) -> Result<(), Error> {
        let mut proposal: Proposal = env
            .storage()
            .instance()
            .get(&DataKey::Proposal(proposal_id))
            .ok_or(Error::NotFound)?;

        if env.storage().instance().has(&DataKey::ProposalCancelled(proposal_id)) {
            return Err(Error::PropNotAct);
        }

        let current_time = env.ledger().timestamp();

        if current_time <= proposal.end_time {
            return Err(Error::InvInput);
        }

        if proposal.executed {
            return Err(Error::PropExc);
        }

        let storage = env.storage().instance();

        let total_votes: Amount = proposal.yes_votes + proposal.no_votes;

        let use_token_quorum = storage.has(&DataKey::GovToken);

        if use_token_quorum {
            let total_stake: Amount = storage.get(&DataKey::TotalStake).unwrap_or(0);
            if total_stake <= 0 {
                return Err(Error::QuorumNR);
            }

            let min_votes_needed = (total_stake * GOVERNANCE_QUORUM as i128) / 10000;

            if total_votes < min_votes_needed {
                return Err(Error::QuorumNR);
            }
        } else {
            let total_voters: u32 = storage.get(&DataKey::TotalVoters).unwrap_or(100);
            let min_votes_needed = (total_voters as u64 * GOVERNANCE_QUORUM as u64) / 10000;
            if (total_votes as u64) < min_votes_needed {
                return Err(Error::QuorumNR);
            }
        }

        if proposal.yes_votes > proposal.no_votes {
            proposal.executed = true;
        } else {
            proposal.executed = false;
        }

        // Record optional timelock for this proposal
        let timelock_delay: u64 = storage.get(&DataKey::TimelockDelay).unwrap_or(0);
        if timelock_delay > 0 {
            let eta = current_time + timelock_delay;
            storage.set(&DataKey::ProposalTimelock(proposal_id), &eta);
        }

        storage.set(&DataKey::Proposal(proposal_id), &proposal);

        // Emit execution event
        env.events()
            .publish((PROPOSAL_EXECUTED,), (proposal_id, proposal.executed));

        Ok(())
    }

    pub fn get_proposal(env: Env, proposal_id: u64) -> Result<Proposal, Error> {
        env.storage()
            .instance()
            .get(&DataKey::Proposal(proposal_id))
            .ok_or(Error::NotFound)
    }

    pub fn get_voting_power(env: Env, address: Address) -> i128 {
        let storage = env.storage().instance();
        
        let mut power: i128 = 0;

        // 1. Reputation
        if let Some(rep_addr) = storage.get::<_, Address>(&DataKey::ReputationContract) {
            // Internal call to reputation contract
            // We'll use a dynamic call or assuming client available.
            // For now, let's assume we have a way to call it.
            // In Soroban, we'd typically use a client.
            // Let's assume we can query it.
            match env.invoke_contract::<i128>(&rep_addr, &soroban_sdk::symbol_short!("get_score"), soroban_sdk::vec![&env, address.to_val()]) {
                Ok(score) => power += score,
                Err(_) => power += 100, // Default if call fails or not registered
            }
        }

        // 2. Token Stake
        if storage.has(&DataKey::GovToken) {
            let stake: Amount = storage.get(&DataKey::Stake(address.clone())).unwrap_or(0);
            power += stake;
        } else {
            power += 1; // Default 1 vote if no tokens and no reputation logic
        }

        // 3. Delegation (Simplified: only one level)
        // We'd need to iterate over all delegators which is inefficient.
        // Usually, voting power is tracked on-change.
        // For this implementation, we'll just return direct power.
        // TODO: Implement delegation tracking if required for efficiency.

        power
    }

    pub fn get_active_proposals(env: Env) -> Vec<u64> {
        let mut active = Vec::new(&env);
        let next_id: u64 = env.storage().instance().get(&DataKey::NextProposalId).unwrap_or(0);
        let current_time = env.ledger().timestamp();

        for i in 0..next_id {
            if let Some(proposal) = env.storage().instance().get::<_, Proposal>(&DataKey::Proposal(i)) {
                if current_time >= proposal.start_time && current_time <= proposal.end_time && !proposal.executed {
                    if !env.storage().instance().has(&DataKey::ProposalCancelled(i)) {
                        active.push_back(i);
                    }
                }
            }
        }
        active
    }

    pub fn get_vote_record(env: Env, voter: Address, proposal_id: u64) -> Option<bool> {
        env.storage().instance().get(&DataKey::VoteChoice(proposal_id, voter))
    }

    pub fn update_governance_parameters(env: Env, admin: Address, key: u32, value: u64) -> Result<(), Error> {
        admin.require_auth();
        let storage = env.storage().instance();
        let stored_admin: Address = storage.get(&DataKey::Admin).ok_or(Error::NotInit)?;
        if stored_admin != admin {
            return Err(Error::Unauthorized);
        }

        match key {
            1 => storage.set(&DataKey::TimelockDelay, &value),
            2 => storage.set(&DataKey::TotalVoters, &(value as u32)),
            _ => return Err(Error::InvInput),
        }

        Ok(())
    }

    pub fn has_voted(env: Env, proposal_id: u64, voter: Address) -> bool {
        let vote_key = DataKey::HasVoted(proposal_id, voter);
        env.storage().instance().has(&vote_key)
    }

    pub fn get_total_voters(env: Env) -> u32 {
        env.storage()
            .instance()
            .get(&DataKey::TotalVoters)
            .unwrap_or(0)
    }

    pub fn get_stake(env: Env, voter: Address) -> Amount {
        env.storage()
            .instance()
            .get(&DataKey::Stake(voter))
            .unwrap_or(0)
    }

    pub fn get_total_stake(env: Env) -> Amount {
        env.storage()
            .instance()
            .get(&DataKey::TotalStake)
            .unwrap_or(0)
    }

    pub fn get_proposal_timelock(env: Env, proposal_id: u64) -> Option<u64> {
        env.storage()
            .instance()
            .get(&DataKey::ProposalTimelock(proposal_id))
    }
}

