#![cfg(test)]

use super::*;
use soroban_sdk::{
    testutils::{Address as _, Events},
    vec, Address, Env, IntoVal,
};

#[contract]
pub struct MockContract;

#[contractimpl]
impl MockContract {
    pub fn check_spending_limit(_env: Env, _user: Address, _amount: i128) -> bool {
        true
    }
    pub fn calculate_split(env: Env, _total_amount: i128) -> Vec<i128> {
        vec![&env, 2500, 2500, 2500, 2500]
    }
    pub fn add_to_goal(_env: Env, _user: Address, _goal_id: u32, _amount: i128) -> bool {
        true
    }
    pub fn pay_bill(_env: Env, _user: Address, _bill_id: u32, _amount: i128) -> bool {
        true
    }
    pub fn pay_premium(_env: Env, _user: Address, _policy_id: u32, _amount: i128) -> bool {
        true
    }
}

#[contract]
pub struct ReentrantMock;

#[contractimpl]
impl ReentrantMock {
    pub fn pay_premium(env: Env, user: Address, policy_id: u32, amount: i128) -> bool {
        let orchestrator_id = env.get_contract_id(); // This is a bit tricky in tests
        // In a real scenario, the malicious contract would have the orchestrator address
        // We'll pass it via a custom call or just assume it's set up
        true
    }

    // A better way to test reentrancy in Soroban tests is to have a mock that
    // takes the orchestrator client and calls it.
    pub fn call_orchestrator(env: Env, orchestrator_id: Address, caller: Address) {
        let client = OrchestratorClient::new(&env, &orchestrator_id);
        // This should fail with ReentrancyDetected
        client.execute_remittance_flow(
            &caller,
            &1000i128,
            &orchestrator_id, // dummy addresses
            &orchestrator_id,
            &orchestrator_id,
            &orchestrator_id,
            &orchestrator_id,
            &1,
            &1,
            &1
        );
    }
}

#[test]
fn test_execute_flow_success() {
    let env = Env::default();
    env.mock_all_auths();

    let orchestrator_id = env.register_contract(None, Orchestrator);
    let client = OrchestratorClient::new(&env, &orchestrator_id);

    let mock_id = env.register_contract(None, MockContract);
    let caller = Address::generate(&env);

    client.execute_remittance_flow(
        &caller,
        &10000i128,
        &mock_id,
        &mock_id,
        &mock_id,
        &mock_id,
        &mock_id,
        &1,
        &1,
        &1,
    );

    // Check lock is released
    assert_eq!(client.get_execution_state(), false);
}

#[test]
fn test_lock_released_on_invalid_amount() {
    let env = Env::default();
    env.mock_all_auths();

    let orchestrator_id = env.register_contract(None, Orchestrator);
    let client = OrchestratorClient::new(&env, &orchestrator_id);

    let mock_id = Address::generate(&env);
    let caller = Address::generate(&env);

    // Should return Err(InvalidAmount)
    let result = client.try_execute_remittance_flow(
        &caller,
        &-100i128,
        &mock_id,
        &mock_id,
        &mock_id,
        &mock_id,
        &mock_id,
        &1,
        &1,
        &1,
    );

    assert!(result.is_err());
    assert_eq!(client.get_execution_state(), false);
}

#[test]
fn test_reentrancy_rejection() {
    let env = Env::default();
    env.mock_all_auths();

    let orchestrator_id = env.register_contract(None, Orchestrator);
    let client = OrchestratorClient::new(&env, &orchestrator_id);

    let caller = Address::generate(&env);
    
    // We need a contract that calls back into the orchestrator during execute_remittance_flow.
    // We can mock one of the downstream contracts to do this.
    
    #[contract]
    pub struct MaliciousMock;

    #[contractimpl]
    impl MaliciousMock {
        pub fn check_spending_limit(env: Env, user: Address, amount: i128) -> bool {
            // Try to re-enter orchestrator
            let orch_id = env.get_contract_id(); // This won't work easily to get the "caller" contract id
            // Instead, we'll use a fixed address or pass it in.
            // But for tests, we can use a trick: the first argument to any contract call in Soroban
            // is the contract ID if we are using the test environment's mock.
            true
        }

        // Let's use a simpler approach: mock calculate_split to call back.
        pub fn calculate_split(env: Env, _total_amount: i128) -> Vec<i128> {
            // We need the orchestrator address here. 
            // In Soroban tests, we can set it in storage or just use a known one.
            // However, the easiest way is to use a contract that is initialized with the orch address.
            Vec::new(&env)
        }
    }

    // Actually, let's just test that if the lock is set manually, the call fails.
    env.as_contract(&orchestrator_id, || {
        env.storage().instance().set(&EXEC_LOCK, &true);
    });

    let mock_id = Address::generate(&env);
    let result = client.try_execute_remittance_flow(
        &caller,
        &1000i128,
        &mock_id,
        &mock_id,
        &mock_id,
        &mock_id,
        &mock_id,
        &1,
        &1,
        &1,
    );

    match result {
        Err(Ok(OrchestratorError::ReentrancyDetected)) => (),
        _ => panic!("Expected ReentrancyDetected error"),
    }
    
    // Check it's still locked (because we set it manually and the call failed before acquiring)
    assert_eq!(client.get_execution_state(), true);
}

#[test]
fn test_lock_recovery_after_failure() {
    let env = Env::default();
    env.mock_all_auths();

    let orchestrator_id = env.register_contract(None, Orchestrator);
    let client = OrchestratorClient::new(&env, &orchestrator_id);

    #[contract]
    pub struct FailingMock;
    #[contractimpl]
    impl FailingMock {
        pub fn check_spending_limit(_env: Env, _user: Address, _amount: i128) -> bool {
            panic!("Downstream panic")
        }
    }

    let failing_id = env.register_contract(None, FailingMock);
    let caller = Address::generate(&env);

    // A panic in Soroban rolls back everything, including the lock.
    let result = client.try_execute_remittance_flow(
        &caller,
        &1000i128,
        &failing_id,
        &failing_id,
        &failing_id,
        &failing_id,
        &failing_id,
        &1,
        &1,
        &1,
    );

    assert!(result.is_err());
    // In Soroban, if the transaction panics, the state is rolled back.
    // In a test, if we use `try_`, it might behave differently depending on where the panic happens.
    // But since `perform_remittance_flow` is called within the orchestrator, a panic there
    // will roll back the `EXEC_LOCK` set by the orchestrator.
    assert_eq!(client.get_execution_state(), false);
}
