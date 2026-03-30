#![cfg(test)]

use crate::{EmergencyWithdrawStatus, EscrowContract, EscrowContractClient};
use shared::types::{DisputeResolution, MilestoneStatus};
use soroban_sdk::{
    contract, contractimpl,
    testutils::{Address as _, Ledger},
    Address, BytesN, Env, Vec,
};

#[contract]
pub struct MockToken;

#[contractimpl]
impl MockToken {
    pub fn transfer(_env: Env, _from: Address, _to: Address, _amount: i128) {}
    pub fn balance(_env: Env, _id: Address) -> i128 {
        1200
    }
}

#[contract]
pub struct MockProfitDist;

#[contractimpl]
impl MockProfitDist {
    pub fn deposit_profits(
        _env: Env,
        _project_id: u64,
        _depositor: Address,
        _amount: i128,
    ) -> Result<(), shared::errors::Error> {
        Ok(())
    }
}

#[contract]
pub struct MockYieldPool;

#[contractimpl]
impl MockYieldPool {
    pub fn deposit(_env: Env, _from: Address, _amount: i128) {}

    pub fn withdraw(_env: Env, _to: Address, _amount: i128) {}

    pub fn get_balance(_env: Env, _account: Address) -> i128 {
        750
    }
}

fn create_mock_token(env: &Env) -> Address {
    env.register_contract(None, MockToken)
}

fn create_mock_yield_pool(env: &Env) -> Address {
    env.register_contract(None, MockYieldPool)
}

fn create_test_env() -> (Env, Address, Address, Address, Vec<Address>) {
    let env = Env::default();
    env.ledger().set_timestamp(1000);

    let creator = Address::generate(&env);
    let token = create_mock_token(&env);
    let validator1 = Address::generate(&env);
    let validator2 = Address::generate(&env);
    let validator3 = Address::generate(&env);

    let mut validators = Vec::new(&env);
    validators.push_back(validator1);
    validators.push_back(validator2);
    validators.push_back(validator3.clone());

    (env, creator, token, validator3, validators)
}

fn create_client(env: &Env) -> EscrowContractClient<'_> {
    EscrowContractClient::new(env, &env.register_contract(None, EscrowContract))
}

fn setup_with_admin(
    env: &Env,
) -> (
    Address,
    Address,
    Address,
    Vec<Address>,
    EscrowContractClient<'_>,
) {
    let admin = Address::generate(env);
    let creator = Address::generate(env);
    let token = create_mock_token(env);

    let mut validators = Vec::new(env);
    validators.push_back(Address::generate(env));
    validators.push_back(Address::generate(env));
    validators.push_back(Address::generate(env));

    let contract_id = env.register_contract(None, EscrowContract);
    let client = EscrowContractClient::new(env, &contract_id);

    client.initialize_admin(&admin);
    client.initialize(&1, &creator, &token, &validators, &DEFAULT_THRESHOLD, &0, &DEFAULT_FEE);

    (admin, creator, token, validators, client)
}

/// Default threshold used by all existing tests (67%).
const DEFAULT_THRESHOLD: u32 = 6700;
const VESTING_DURATION: u64 = 3600;
const DEFAULT_FEE: u32 = 500;

#[test]
fn test_initialize_escrow() {
    let (env, creator, token, _, validators) = create_test_env();
    let client = create_client(&env);
    env.mock_all_auths();

    client.initialize(&1, &creator, &token, &validators, &DEFAULT_THRESHOLD, &VESTING_DURATION, &DEFAULT_FEE);

    let escrow = client.get_escrow(&1);
    assert_eq!(escrow.project_id, 1);
    assert_eq!(escrow.creator, creator);
    assert_eq!(escrow.token, token);
    assert_eq!(escrow.total_deposited, 0);
    assert_eq!(escrow.released_amount, 0);
    assert_eq!(escrow.approval_threshold, DEFAULT_THRESHOLD);
    assert_eq!(escrow.vesting_duration, VESTING_DURATION);
    assert_eq!(escrow.management_fee_bps, DEFAULT_FEE);
}

#[test]
fn test_initialize_with_vesting() {
    let (env, creator, token, _, validators) = create_test_env();
    let client = create_client(&env);
    env.mock_all_auths();

    client.initialize(&1, &creator, &token, &validators, &DEFAULT_THRESHOLD, &VESTING_DURATION, &0);

    let escrow = client.get_escrow(&1);
    assert_eq!(escrow.vesting_duration, VESTING_DURATION);
}

#[test]
fn test_initialize_with_insufficient_validators() {
    let env = Env::default();
    let creator = Address::generate(&env);
    let token = Address::generate(&env);

    let mut validators = Vec::new(&env);
    validators.push_back(Address::generate(&env));

    let client = create_client(&env);
    let result = client.try_initialize(&1, &creator, &token, &validators, &DEFAULT_THRESHOLD, &0, &0);

    assert!(result.is_err());
}

#[test]
fn test_initialize_duplicate_escrow() {
    let (env, creator, token, _, validators) = create_test_env();
    let client = create_client(&env);
    env.mock_all_auths();

    client.initialize(&1, &creator, &token, &validators, &DEFAULT_THRESHOLD, &0, &0);

    let result = client.try_initialize(&1, &creator, &token, &validators, &DEFAULT_THRESHOLD, &0, &0);
    assert!(result.is_err());
}

#[test]
fn test_deposit_funds() {
    let (env, creator, token, _, validators) = create_test_env();
    let client = create_client(&env);
    env.mock_all_auths();

    client.initialize(&1, &creator, &token, &validators, &DEFAULT_THRESHOLD, &0, &0);

    let deposit_amount: i128 = 1000;
    client.deposit(&1, &deposit_amount);

    let escrow = client.get_escrow(&1);
    assert_eq!(escrow.total_deposited, deposit_amount);
}

#[test]
fn test_create_milestone() {
    let (env, creator, token, _, validators) = create_test_env();
    let client = create_client(&env);

    env.mock_all_auths();
    client.initialize(&1, &creator, &token, &validators, &DEFAULT_THRESHOLD, &0, &0);
    client.deposit(&1, &1000);

    let description_hash = BytesN::from_array(&env, &[1u8; 32]);
    client.create_milestone(&1, &description_hash, &500);

    let milestone = client.get_milestone(&1, &0);
    assert_eq!(milestone.id, 0);
    assert_eq!(milestone.project_id, 1);
    assert_eq!(milestone.amount, 500);
    assert_eq!(milestone.status, MilestoneStatus::Pending);
}

#[test]
fn test_submit_milestone() {
    let (env, creator, token, _, validators) = create_test_env();
    let client = create_client(&env);

    env.mock_all_auths();
    client.initialize(&1, &creator, &token, &validators, &DEFAULT_THRESHOLD, &0, &0);
    client.deposit(&1, &1000);

    let description_hash = BytesN::from_array(&env, &[1u8; 32]);
    client.create_milestone(&1, &description_hash, &500);

    let proof_hash = BytesN::from_array(&env, &[9u8; 32]);
    client.submit_milestone(&1, &0, &proof_hash);

    let milestone = client.get_milestone(&1, &0);
    assert_eq!(milestone.status, MilestoneStatus::Submitted);
    assert_eq!(milestone.proof_hash, proof_hash);
}

#[test]
fn test_claim_vested_milestone_happy_path() {
    let (env, creator, token, _, validators) = create_test_env();
    let client = create_client(&env);
    env.mock_all_auths();

    client.initialize(&1, &creator, &token, &validators, &DEFAULT_THRESHOLD, &VESTING_DURATION, &0);
    client.deposit(&1, &1000);

    let description_hash = BytesN::from_array(&env, &[1u8; 32]);
    client.create_milestone(&1, &description_hash, &500);

    let proof_hash = BytesN::from_array(&env, &[9u8; 32]);
    client.submit_milestone(&1, &0, &proof_hash);

    client.vote_milestone(&1, &0, &validators.get(0).unwrap(), &true);
    client.vote_milestone(&1, &0, &validators.get(1).unwrap(), &true);

    assert_eq!(client.get_milestone(&1, &0).status, MilestoneStatus::Approved);

    // Fast forward halfway through vesting
    env.ledger().set_timestamp(1000 + VESTING_DURATION / 2);
    
    // Should be able to claim 50% = 250
    client.claim_milestone(&1, &0);
    
    let escrow = client.get_escrow(&1);
    assert_eq!(escrow.released_amount, 250);
}

#[test]
fn test_claim_vested_milestone_partial() {
    let (env, creator, token, _, validators) = create_test_env();
    let client = create_client(&env);
    env.mock_all_auths();

    client.initialize(&1, &creator, &token, &validators, &DEFAULT_THRESHOLD, &VESTING_DURATION, &0);
    client.deposit(&1, &1000);

    let description_hash = BytesN::from_array(&env, &[1u8; 32]);
    client.create_milestone(&1, &description_hash, &500);

    let proof_hash = BytesN::from_array(&env, &[9u8; 32]);
    client.submit_milestone(&1, &0, &proof_hash);

    client.vote_milestone(&1, &0, &validators.get(0).unwrap(), &true);
    client.vote_milestone(&1, &0, &validators.get(1).unwrap(), &true);

    // Quarter way through
    env.ledger().set_timestamp(1000 + VESTING_DURATION / 4);
    client.claim_milestone(&1, &0); // 125
    
    // Half way through (total 50%)
    env.ledger().set_timestamp(1000 + VESTING_DURATION / 2);
    client.claim_milestone(&1, &0); // Another 125
    
    let escrow = client.get_escrow(&1);
    assert_eq!(escrow.released_amount, 250);
}

#[test]
fn test_claim_milestone_no_vesting() {
    let (env, creator, token, _, validators) = create_test_env();
    let client = create_client(&env);
    env.mock_all_auths();

    client.initialize(&1, &creator, &token, &validators, &DEFAULT_THRESHOLD, &0, &0);
    client.deposit(&1, &1000);

    let description_hash = BytesN::from_array(&env, &[1u8; 32]);
    client.create_milestone(&1, &description_hash, &500);

    let proof_hash = BytesN::from_array(&env, &[9u8; 32]);
    client.submit_milestone(&1, &0, &proof_hash);

    client.vote_milestone(&1, &0, &validators.get(0).unwrap(), &true);
    client.vote_milestone(&1, &0, &validators.get(1).unwrap(), &true);

    // Claim immediately
    client.claim_milestone(&1, &0);
    
    let escrow = client.get_escrow(&1);
    assert_eq!(escrow.released_amount, 500);
}

#[test]
fn test_claim_yield_happy_path() {
    let (env, creator, token, _, validators) = create_test_env();
    let client = create_client(&env);
    env.mock_all_auths();

    let profit_dist_id = env.register_contract(None, MockProfitDist);
    
    client.initialize(&1, &creator, &token, &validators, &DEFAULT_THRESHOLD, &0, &500);
    client.deposit(&1, &1000);
    
    // MockToken::balance returns 1200. Total deposited is 1000. Yield is 200.
    // Fee is 5% of 200 = 10.
    client.claim_yield(&1, &profit_dist_id);
}

#[test]
fn test_pause_blocks_deposit() {
    let env = Env::default();
    env.ledger().set_timestamp(1000);
    env.mock_all_auths();
    let (admin, _, _, _, client) = setup_with_admin(&env);

    client.pause(&admin);

    let result = client.try_deposit(&1, &500);
    assert!(result.is_err(), "deposit should be blocked when paused");
}

#[test]
fn test_resume_after_time_delay_succeeds() {
    let env = Env::default();
    env.ledger().set_timestamp(1000);
    env.mock_all_auths();
    let (admin, _, _, _, client) = setup_with_admin(&env);

    client.pause(&admin);

    env.ledger().set_timestamp(1000 + 86400 + 1);
    let result = client.try_resume(&admin);
    assert!(result.is_ok(), "resume should succeed after time delay");
    assert!(!client.get_is_paused());
}

#[test]
fn test_emergency_withdraw_from_pool() {
    let env = Env::default();
    env.ledger().set_timestamp(1000);
    env.mock_all_auths();
    let (admin, _, _, validators, client) = setup_with_admin(&env);

    let yield_pool = create_mock_yield_pool(&env);
    
    // Project 1: Configure yield
    client.configure_yield_router(&1, &yield_pool);
    client.enable_yield_for_escrow(&1);

    // Request emergency withdraw
    client.request_emergency_withdraw(&1);
    
    // Need majority approvals (with 3 validators, need 2)
    client.approve_emergency_withdraw(&1, &validators.get(0).unwrap());
    client.approve_emergency_withdraw(&1, &validators.get(1).unwrap());
    
    // Execute
    client.execute_emergency_withdraw(&1);
    
    let state = client.get_emergency_withdraw_state(&1);
    assert_eq!(state.status, EmergencyWithdrawStatus::Executed);
}
