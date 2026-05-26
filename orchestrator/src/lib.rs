#![no_std]

use soroban_sdk::{
    contract, contracterror, contractimpl, symbol_short, Address, Env, Symbol, Vec,
};

mod interface {
    use soroban_sdk::{contractclient, Address, Env, Vec};

    #[contractclient(name = "FamilyWalletClient")]
    pub trait FamilyWalletInterface {
        fn check_spending_limit(env: Env, user: Address, amount: i128) -> bool;
    }

    #[contractclient(name = "RemittanceSplitClient")]
    pub trait RemittanceSplitInterface {
        fn calculate_split(env: Env, total_amount: i128) -> Vec<i128>;
    }

    #[contractclient(name = "SavingsGoalsClient")]
    pub trait SavingsGoalsInterface {
        fn add_to_goal(env: Env, user: Address, goal_id: u32, amount: i128) -> bool;
    }

    #[contractclient(name = "BillPaymentsClient")]
    pub trait BillPaymentsInterface {
        fn pay_bill(env: Env, user: Address, bill_id: u32, amount: i128) -> bool;
    }

    #[contractclient(name = "InsuranceClient")]
    pub trait InsuranceInterface {
        fn pay_premium(env: Env, user: Address, policy_id: u32, amount: i128) -> bool;
    }
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum OrchestratorError {
    ReentrancyDetected = 10,
    PermissionDenied = 11,
    InvalidAmount = 12,
}

#[contracttype]
#[derive(Clone)]
pub struct OrchestratorAuditEntry {
    pub operation: Symbol,
    pub caller: Address,
    pub timestamp: u64,
    pub success: bool,
}

const EXEC_LOCK: Symbol = symbol_short!("EXEC_LOCK");
const AUDIT: Symbol = symbol_short!("AUDIT");
const MAX_AUDIT_ENTRIES: u32 = 100;

/// RAII guard to ensure the execution lock is released on drop.
pub struct LockGuard {
    env: Env,
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        self.env.storage().instance().set(&EXEC_LOCK, &false);
    }
}

#[contract]
pub struct Orchestrator;

#[contractimpl]
impl Orchestrator {
    /// Executes the full remittance flow across multiple contracts.
    /// This is protected against reentrancy.
    pub fn execute_remittance_flow(
        env: Env,
        caller: Address,
        total_amount: i128,
        family_wallet: Address,
        remittance_split: Address,
        savings: Address,
        bills: Address,
        insurance: Address,
        goal_id: u32,
        bill_id: u32,
        policy_id: u32,
    ) -> Result<(), OrchestratorError> {
        caller.require_auth();

        if total_amount <= 0 {
            return Err(OrchestratorError::InvalidAmount);
        }

        // Use a scope to ensure the guard is dropped (and lock released) 
        // before we audit and return.
        let result = {
            /// The guard acquires the lock on creation and releases it on drop.
            /// This ensures the lock is released even if we return early via `?`.
            let _guard = Self::acquire_execution_lock(&env)?;

            Self::perform_remittance_flow(
                &env,
                &caller,
                total_amount,
                &family_wallet,
                &remittance_split,
                &savings,
                &bills,
                &insurance,
                goal_id,
                bill_id,
                policy_id,
            )
        };

        // 4. Audit result (lock is already released here)
        Self::append_audit(&env, symbol_short!("remit"), &caller, result.is_ok());

        result
    }

    fn perform_remittance_flow(
        env: &Env,
        caller: &Address,
        total_amount: i128,
        family_wallet: &Address,
        remittance_split: &Address,
        savings: &Address,
        bills: &Address,
        insurance: &Address,
        goal_id: u32,
        bill_id: u32,
        policy_id: u32,
    ) -> Result<(), OrchestratorError> {
        // Use interfaces to call downstream contracts
        // This is a simplified implementation of the flow logic
        
        // 1. Check permission/spending limit
        let fw_client = interface::FamilyWalletClient::new(env, family_wallet);
        if !fw_client.check_spending_limit(caller, &total_amount) {
            return Err(OrchestratorError::PermissionDenied);
        }

        // 2. Calculate split
        let rs_client = interface::RemittanceSplitClient::new(env, remittance_split);
        let allocations = rs_client.calculate_split(&total_amount);
        
        if allocations.len() < 4 {
            return Err(OrchestratorError::InvalidAmount);
        }

        let _spending_amt = allocations.get_unchecked(0);
        let savings_amt = allocations.get_unchecked(1);
        let bills_amt = allocations.get_unchecked(2);
        let insurance_amt = allocations.get_unchecked(3);

        // 3. Downstream calls
        if savings_amt > 0 {
            let s_client = interface::SavingsGoalsClient::new(env, savings);
            s_client.add_to_goal(caller, &goal_id, &savings_amt);
        }

        if bills_amt > 0 {
            let b_client = interface::BillPaymentsClient::new(env, bills);
            b_client.pay_bill(caller, &bill_id, &bills_amt);
        }

        if insurance_amt > 0 {
            let i_client = interface::InsuranceClient::new(env, insurance);
            i_client.pay_premium(caller, &policy_id, &insurance_amt);
        }

        Ok(())
    }

    fn acquire_execution_lock(env: &Env) -> Result<LockGuard, OrchestratorError> {
        let is_locked: bool = env.storage().instance().get(&EXEC_LOCK).unwrap_or(false);
        if is_locked {
            return Err(OrchestratorError::ReentrancyDetected);
        }
        env.storage().instance().set(&EXEC_LOCK, &true);
        Ok(LockGuard { env: env.clone() })
    }

    fn append_audit(env: &Env, operation: Symbol, caller: &Address, success: bool) {
        let timestamp = env.ledger().timestamp();
        let mut log: Vec<OrchestratorAuditEntry> = env
            .storage()
            .instance()
            .get(&AUDIT)
            .unwrap_or_else(|| Vec::new(env));
        
        if log.len() >= MAX_AUDIT_ENTRIES {
            log.remove(0);
        }
        
        log.push_back(OrchestratorAuditEntry {
            operation,
            caller: caller.clone(),
            timestamp,
            success,
        });
        
        env.storage().instance().set(&AUDIT, &log);
    }

    pub fn get_execution_state(env: Env) -> bool {
        env.storage().instance().get(&EXEC_LOCK).unwrap_or(false)
    }
}
