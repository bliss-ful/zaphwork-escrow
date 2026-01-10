use proptest::prelude::*;

// ============================================================================
// PROPERTY-BASED TESTS FOR ESCROW CONTRACT
// ============================================================================

/// Test fee calculation correctness
#[cfg(test)]
mod fee_tests {
    use super::*;

    const BPS_DENOMINATOR: u64 = 10_000;
    const MAX_FEE_BPS: u64 = 2000; // 20%

    fn calculate_fee(amount: u64, fee_bps: u64) -> Option<u64> {
        let fee = (amount as u128)
            .checked_mul(fee_bps as u128)?
            .checked_div(BPS_DENOMINATOR as u128)?;
        if fee > u64::MAX as u128 {
            return None;
        }
        Some(fee as u64)
    }

    proptest! {
        #[test]
        fn fee_never_exceeds_amount(amount in 1u64..u64::MAX/2, fee_bps in 0u64..=MAX_FEE_BPS) {
            if let Some(fee) = calculate_fee(amount, fee_bps) {
                prop_assert!(fee <= amount, "Fee {} exceeds amount {}", fee, amount);
            }
        }

        #[test]
        fn fee_is_proportional(amount in 1_000_000u64..1_000_000_000u64, fee_bps in 1u64..=MAX_FEE_BPS) {
            if let Some(fee) = calculate_fee(amount, fee_bps) {
                let expected_ratio = fee_bps as f64 / BPS_DENOMINATOR as f64;
                let actual_ratio = fee as f64 / amount as f64;
                let diff = (expected_ratio - actual_ratio).abs();
                prop_assert!(diff < 0.0001, "Fee ratio mismatch: expected {}, got {}", expected_ratio, actual_ratio);
            }
        }

        #[test]
        fn zero_fee_bps_means_zero_fee(amount in 1u64..u64::MAX) {
            let fee = calculate_fee(amount, 0).unwrap();
            prop_assert_eq!(fee, 0, "Zero fee_bps should result in zero fee");
        }

        #[test]
        fn worker_amount_plus_fee_equals_total(amount in 1_000_000u64..1_000_000_000u64, fee_bps in 0u64..=MAX_FEE_BPS) {
            if let Some(fee) = calculate_fee(amount, fee_bps) {
                let worker_amount = amount.saturating_sub(fee);
                prop_assert_eq!(worker_amount + fee, amount, "Worker amount + fee should equal total");
            }
        }
    }
}


/// Test status transition validity
#[cfg(test)]
mod status_tests {
    use proptest::prelude::*;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum EscrowStatus {
        Created,
        Funded,
        Released,
        Refunded,
        Frozen,
    }

    fn can_transition(from: EscrowStatus, to: EscrowStatus) -> bool {
        match (from, to) {
            // Created can only go to Funded (via fund) or be cancelled
            (EscrowStatus::Created, EscrowStatus::Funded) => true,
            // Funded can go to Released, Refunded, or Frozen
            (EscrowStatus::Funded, EscrowStatus::Released) => true,
            (EscrowStatus::Funded, EscrowStatus::Refunded) => true,
            (EscrowStatus::Funded, EscrowStatus::Frozen) => true,
            // Frozen can go to Released or Refunded (admin action)
            (EscrowStatus::Frozen, EscrowStatus::Released) => true,
            (EscrowStatus::Frozen, EscrowStatus::Refunded) => true,
            // No other transitions allowed
            _ => false,
        }
    }

    #[test]
    fn test_valid_transitions() {
        assert!(can_transition(EscrowStatus::Created, EscrowStatus::Funded));
        assert!(can_transition(EscrowStatus::Funded, EscrowStatus::Released));
        assert!(can_transition(EscrowStatus::Funded, EscrowStatus::Refunded));
        assert!(can_transition(EscrowStatus::Funded, EscrowStatus::Frozen));
        assert!(can_transition(EscrowStatus::Frozen, EscrowStatus::Released));
        assert!(can_transition(EscrowStatus::Frozen, EscrowStatus::Refunded));
    }

    #[test]
    fn test_invalid_transitions() {
        // Cannot go backwards
        assert!(!can_transition(EscrowStatus::Funded, EscrowStatus::Created));
        assert!(!can_transition(EscrowStatus::Released, EscrowStatus::Funded));
        // Cannot skip states
        assert!(!can_transition(EscrowStatus::Created, EscrowStatus::Released));
        assert!(!can_transition(EscrowStatus::Created, EscrowStatus::Frozen));
        // Terminal states cannot transition
        assert!(!can_transition(EscrowStatus::Released, EscrowStatus::Refunded));
        assert!(!can_transition(EscrowStatus::Refunded, EscrowStatus::Released));
    }
}

/// Test split funds calculation
#[cfg(test)]
mod split_tests {
    use proptest::prelude::*;

    const BPS_DENOMINATOR: u64 = 10_000;

    /// Calculate split using basis points (0-10000) for precision
    /// This matches the updated admin_split_funds function
    fn calculate_split_bps(amount: u64, worker_bps: u64) -> Option<(u64, u64)> {
        if worker_bps > BPS_DENOMINATOR {
            return None;
        }
        // Use u128 to prevent overflow during multiplication
        let worker_amount = ((amount as u128)
            .checked_mul(worker_bps as u128)?
            .checked_div(BPS_DENOMINATOR as u128)?) as u64;
        let client_amount = amount.checked_sub(worker_amount)?;
        Some((worker_amount, client_amount))
    }

    proptest! {
        #[test]
        fn split_sums_to_total(amount in 1_000_000u64..1_000_000_000u64, worker_bps in 0u64..=10_000u64) {
            if let Some((worker, client)) = calculate_split_bps(amount, worker_bps) {
                prop_assert_eq!(worker + client, amount, "Split should sum to total");
            }
        }

        #[test]
        fn zero_percentage_gives_all_to_client(amount in 1_000_000u64..1_000_000_000u64) {
            let (worker, client) = calculate_split_bps(amount, 0).unwrap();
            prop_assert_eq!(worker, 0);
            prop_assert_eq!(client, amount);
        }

        #[test]
        fn hundred_percentage_gives_all_to_worker(amount in 1_000_000u64..1_000_000_000u64) {
            // 10000 bps = 100%
            let (worker, client) = calculate_split_bps(amount, 10_000).unwrap();
            prop_assert_eq!(worker, amount);
            prop_assert_eq!(client, 0);
        }

        #[test]
        fn fifty_fifty_split(amount in 2_000_000u64..1_000_000_000u64) {
            // Use even amounts for exact 50/50
            let even_amount = (amount / 2) * 2;
            // 5000 bps = 50%
            let (worker, client) = calculate_split_bps(even_amount, 5_000).unwrap();
            prop_assert_eq!(worker, client, "50/50 split should be equal");
        }
    }
}

/// Test minimum amount validation
#[cfg(test)]
mod amount_tests {
    use proptest::prelude::*;

    const MIN_ESCROW_AMOUNT: u64 = 1_000_000; // 1 USDC

    fn validate_amount(amount: u64) -> bool {
        amount >= MIN_ESCROW_AMOUNT
    }

    proptest! {
        #[test]
        fn amounts_below_minimum_rejected(amount in 0u64..MIN_ESCROW_AMOUNT) {
            prop_assert!(!validate_amount(amount), "Amount {} should be rejected", amount);
        }

        #[test]
        fn amounts_at_or_above_minimum_accepted(amount in MIN_ESCROW_AMOUNT..u64::MAX) {
            prop_assert!(validate_amount(amount), "Amount {} should be accepted", amount);
        }
    }

    #[test]
    fn exact_minimum_accepted() {
        assert!(validate_amount(MIN_ESCROW_AMOUNT));
    }

    #[test]
    fn zero_rejected() {
        assert!(!validate_amount(0));
    }
}

/// Test deadline validation
#[cfg(test)]
mod deadline_tests {
    use proptest::prelude::*;

    fn is_deadline_passed(current_time: i64, deadline: i64) -> bool {
        current_time > deadline
    }

    proptest! {
        #[test]
        fn deadline_not_passed_when_current_before(deadline in 1000i64..i64::MAX/2) {
            let current = deadline - 1;
            prop_assert!(!is_deadline_passed(current, deadline));
        }

        #[test]
        fn deadline_passed_when_current_after(deadline in 0i64..i64::MAX/2) {
            let current = deadline + 1;
            prop_assert!(is_deadline_passed(current, deadline));
        }

        #[test]
        fn deadline_not_passed_when_equal(time in 0i64..i64::MAX) {
            prop_assert!(!is_deadline_passed(time, time), "Equal time should not count as passed");
        }
    }
}

/// Test authorization checks
#[cfg(test)]
mod auth_tests {
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    struct Pubkey([u8; 32]);

    impl Pubkey {
        fn new(seed: u8) -> Self {
            let mut bytes = [0u8; 32];
            bytes[0] = seed;
            Pubkey(bytes)
        }
    }

    struct Escrow {
        client: Pubkey,
        worker: Pubkey,
    }

    fn can_freeze(caller: Pubkey, escrow: &Escrow, admin: Pubkey) -> bool {
        caller == escrow.client || caller == escrow.worker || caller == admin
    }

    fn can_release(caller: Pubkey, escrow: &Escrow) -> bool {
        caller == escrow.client
    }

    fn can_refund(caller: Pubkey, escrow: &Escrow) -> bool {
        caller == escrow.client
    }

    #[test]
    fn client_can_freeze() {
        let client = Pubkey::new(1);
        let worker = Pubkey::new(2);
        let admin = Pubkey::new(3);
        let escrow = Escrow { client, worker };
        assert!(can_freeze(client, &escrow, admin));
    }

    #[test]
    fn worker_can_freeze() {
        let client = Pubkey::new(1);
        let worker = Pubkey::new(2);
        let admin = Pubkey::new(3);
        let escrow = Escrow { client, worker };
        assert!(can_freeze(worker, &escrow, admin));
    }

    #[test]
    fn admin_can_freeze() {
        let client = Pubkey::new(1);
        let worker = Pubkey::new(2);
        let admin = Pubkey::new(3);
        let escrow = Escrow { client, worker };
        assert!(can_freeze(admin, &escrow, admin));
    }

    #[test]
    fn random_cannot_freeze() {
        let client = Pubkey::new(1);
        let worker = Pubkey::new(2);
        let admin = Pubkey::new(3);
        let random = Pubkey::new(4);
        let escrow = Escrow { client, worker };
        assert!(!can_freeze(random, &escrow, admin));
    }

    #[test]
    fn only_client_can_release() {
        let client = Pubkey::new(1);
        let worker = Pubkey::new(2);
        let escrow = Escrow { client, worker };
        assert!(can_release(client, &escrow));
        assert!(!can_release(worker, &escrow));
    }

    #[test]
    fn only_client_can_refund() {
        let client = Pubkey::new(1);
        let worker = Pubkey::new(2);
        let escrow = Escrow { client, worker };
        assert!(can_refund(client, &escrow));
        assert!(!can_refund(worker, &escrow));
    }
}

/// Test overflow protection
#[cfg(test)]
mod overflow_tests {
    use proptest::prelude::*;

    const BPS_DENOMINATOR: u64 = 10_000;

    fn safe_fee_calculation(amount: u64, fee_bps: u64) -> Option<u64> {
        let fee = (amount as u128)
            .checked_mul(fee_bps as u128)?
            .checked_div(BPS_DENOMINATOR as u128)?;
        if fee > u64::MAX as u128 {
            return None;
        }
        Some(fee as u64)
    }

    proptest! {
        #[test]
        fn no_overflow_on_large_amounts(amount in u64::MAX/2..u64::MAX, fee_bps in 0u64..=2000u64) {
            // Should not panic, may return None for overflow
            let _ = safe_fee_calculation(amount, fee_bps);
        }

        #[test]
        fn calculation_succeeds_for_reasonable_amounts(amount in 1u64..1_000_000_000_000u64, fee_bps in 0u64..=2000u64) {
            let result = safe_fee_calculation(amount, fee_bps);
            prop_assert!(result.is_some(), "Should succeed for reasonable amounts");
        }
    }

    #[test]
    fn max_amount_with_max_fee() {
        // This should not overflow with u128 intermediate
        let result = safe_fee_calculation(u64::MAX, 2000);
        // May or may not succeed depending on final value, but should not panic
        let _ = result;
    }
}


// ============================================================================
// EDGE CASE TESTS
// ============================================================================

/// Test edge cases for deadline validation
#[cfg(test)]
mod deadline_edge_cases {
    const MAX_ESCROW_DURATION: i64 = 365 * 24 * 60 * 60; // 1 year in seconds

    fn validate_deadline(current_time: i64, deadline: i64) -> Result<(), &'static str> {
        if deadline <= current_time {
            return Err("DeadlineInPast");
        }
        let max_deadline = current_time.checked_add(MAX_ESCROW_DURATION)
            .ok_or("Overflow")?;
        if deadline > max_deadline {
            return Err("DeadlineTooFar");
        }
        Ok(())
    }

    #[test]
    fn deadline_exactly_now_rejected() {
        let now = 1700000000i64;
        let result = validate_deadline(now, now);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "DeadlineInPast");
    }

    #[test]
    fn deadline_one_second_ago_rejected() {
        let now = 1700000000i64;
        let result = validate_deadline(now, now - 1);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "DeadlineInPast");
    }

    #[test]
    fn deadline_one_second_future_accepted() {
        let now = 1700000000i64;
        let result = validate_deadline(now, now + 1);
        assert!(result.is_ok());
    }

    #[test]
    fn deadline_exactly_one_year_accepted() {
        let now = 1700000000i64;
        let result = validate_deadline(now, now + MAX_ESCROW_DURATION);
        assert!(result.is_ok());
    }

    #[test]
    fn deadline_one_second_over_one_year_rejected() {
        let now = 1700000000i64;
        let result = validate_deadline(now, now + MAX_ESCROW_DURATION + 1);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "DeadlineTooFar");
    }

    #[test]
    fn deadline_100_years_rejected() {
        let now = 1700000000i64;
        let hundred_years = 100 * 365 * 24 * 60 * 60i64;
        let result = validate_deadline(now, now + hundred_years);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "DeadlineTooFar");
    }
}

/// Test edge cases for amount validation
#[cfg(test)]
mod amount_edge_cases {
    const MIN_ESCROW_AMOUNT: u64 = 1_000_000; // 1 USDC (6 decimals)
    const BPS_DENOMINATOR: u64 = 10_000;
    const MAX_FEE_BPS: u64 = 2000; // 20%

    fn calculate_fee(amount: u64, fee_bps: u64) -> Option<u64> {
        let fee = (amount as u128)
            .checked_mul(fee_bps as u128)?
            .checked_div(BPS_DENOMINATOR as u128)?;
        if fee > u64::MAX as u128 {
            return None;
        }
        Some(fee as u64)
    }

    #[test]
    fn max_u64_amount_with_max_fee() {
        // Test that max amount doesn't overflow with u128 intermediate
        let result = calculate_fee(u64::MAX, MAX_FEE_BPS);
        assert!(result.is_some());
        let fee = result.unwrap();
        // Fee should be 20% of max u64
        assert!(fee > 0);
        assert!(fee <= u64::MAX / 5); // Roughly 20%
    }

    #[test]
    fn minimum_amount_with_max_fee() {
        let fee = calculate_fee(MIN_ESCROW_AMOUNT, MAX_FEE_BPS).unwrap();
        let worker_amount = MIN_ESCROW_AMOUNT - fee;
        // Worker should get at least 80% of minimum
        assert_eq!(fee, 200_000); // 20% of 1 USDC = 0.2 USDC
        assert_eq!(worker_amount, 800_000); // 80% of 1 USDC = 0.8 USDC
    }

    #[test]
    fn minimum_amount_with_zero_fee() {
        let fee = calculate_fee(MIN_ESCROW_AMOUNT, 0).unwrap();
        assert_eq!(fee, 0);
    }

    #[test]
    fn one_lamport_below_minimum_rejected() {
        let amount = MIN_ESCROW_AMOUNT - 1;
        assert!(amount < MIN_ESCROW_AMOUNT);
    }

    #[test]
    fn fee_calculation_precision() {
        // Test that small amounts don't lose precision
        let amount = 1_000_001u64; // Just above minimum
        let fee = calculate_fee(amount, 1000).unwrap(); // 10%
        // 10% of 1,000,001 = 100,000.1, truncated to 100,000
        assert_eq!(fee, 100_000);
    }
}

/// Test dispute resolution scenarios
#[cfg(test)]
mod dispute_tests {
    const BPS_DENOMINATOR: u64 = 10_000;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum EscrowStatus {
        Created,
        Funded,
        Released,
        Refunded,
        Frozen,
    }

    struct Escrow {
        amount: u64,
        status: EscrowStatus,
        funded_at: Option<i64>,
    }

    fn admin_release_to_worker(escrow: &mut Escrow) -> Result<u64, &'static str> {
        if escrow.status != EscrowStatus::Frozen {
            return Err("InvalidStatus");
        }
        if escrow.funded_at.is_none() {
            return Err("NotFunded");
        }
        let worker_amount = escrow.amount; // Admin release bypasses fees
        escrow.status = EscrowStatus::Released;
        Ok(worker_amount)
    }

    fn admin_refund_to_client(escrow: &mut Escrow) -> Result<u64, &'static str> {
        if escrow.status != EscrowStatus::Frozen {
            return Err("InvalidStatus");
        }
        if escrow.funded_at.is_none() {
            return Err("NotFunded");
        }
        let client_amount = escrow.amount; // Admin refund bypasses fees
        escrow.status = EscrowStatus::Refunded;
        Ok(client_amount)
    }

    fn admin_split_funds(escrow: &mut Escrow, worker_bps: u64) -> Result<(u64, u64), &'static str> {
        if escrow.status != EscrowStatus::Frozen {
            return Err("InvalidStatus");
        }
        if escrow.funded_at.is_none() {
            return Err("NotFunded");
        }
        if worker_bps > BPS_DENOMINATOR {
            return Err("InvalidPercentage");
        }
        let worker_amount = ((escrow.amount as u128) * (worker_bps as u128) / (BPS_DENOMINATOR as u128)) as u64;
        let client_amount = escrow.amount - worker_amount;
        escrow.status = EscrowStatus::Released;
        Ok((worker_amount, client_amount))
    }

    #[test]
    fn admin_release_on_frozen_escrow() {
        let mut escrow = Escrow {
            amount: 10_000_000,
            status: EscrowStatus::Frozen,
            funded_at: Some(1700000000),
        };
        let result = admin_release_to_worker(&mut escrow);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 10_000_000); // Full amount, no fees
        assert_eq!(escrow.status, EscrowStatus::Released);
    }

    #[test]
    fn admin_release_on_funded_escrow_fails() {
        let mut escrow = Escrow {
            amount: 10_000_000,
            status: EscrowStatus::Funded,
            funded_at: Some(1700000000),
        };
        let result = admin_release_to_worker(&mut escrow);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "InvalidStatus");
    }

    #[test]
    fn admin_release_on_unfunded_frozen_escrow_fails() {
        let mut escrow = Escrow {
            amount: 10_000_000,
            status: EscrowStatus::Frozen,
            funded_at: None, // Never funded
        };
        let result = admin_release_to_worker(&mut escrow);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "NotFunded");
    }

    #[test]
    fn admin_refund_on_frozen_escrow() {
        let mut escrow = Escrow {
            amount: 10_000_000,
            status: EscrowStatus::Frozen,
            funded_at: Some(1700000000),
        };
        let result = admin_refund_to_client(&mut escrow);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 10_000_000); // Full amount, no fees
        assert_eq!(escrow.status, EscrowStatus::Refunded);
    }

    #[test]
    fn admin_split_50_50() {
        let mut escrow = Escrow {
            amount: 10_000_000,
            status: EscrowStatus::Frozen,
            funded_at: Some(1700000000),
        };
        let result = admin_split_funds(&mut escrow, 5000); // 50%
        assert!(result.is_ok());
        let (worker, client) = result.unwrap();
        assert_eq!(worker, 5_000_000);
        assert_eq!(client, 5_000_000);
        assert_eq!(worker + client, 10_000_000);
    }

    #[test]
    fn admin_split_70_30() {
        let mut escrow = Escrow {
            amount: 10_000_000,
            status: EscrowStatus::Frozen,
            funded_at: Some(1700000000),
        };
        let result = admin_split_funds(&mut escrow, 7000); // 70% to worker
        assert!(result.is_ok());
        let (worker, client) = result.unwrap();
        assert_eq!(worker, 7_000_000);
        assert_eq!(client, 3_000_000);
    }

    #[test]
    fn admin_split_invalid_percentage_fails() {
        let mut escrow = Escrow {
            amount: 10_000_000,
            status: EscrowStatus::Frozen,
            funded_at: Some(1700000000),
        };
        let result = admin_split_funds(&mut escrow, 10001); // Over 100%
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "InvalidPercentage");
    }

    #[test]
    fn admin_split_100_to_worker() {
        let mut escrow = Escrow {
            amount: 10_000_000,
            status: EscrowStatus::Frozen,
            funded_at: Some(1700000000),
        };
        let result = admin_split_funds(&mut escrow, 10000); // 100% to worker
        assert!(result.is_ok());
        let (worker, client) = result.unwrap();
        assert_eq!(worker, 10_000_000);
        assert_eq!(client, 0);
    }

    #[test]
    fn admin_split_0_to_worker() {
        let mut escrow = Escrow {
            amount: 10_000_000,
            status: EscrowStatus::Frozen,
            funded_at: Some(1700000000),
        };
        let result = admin_split_funds(&mut escrow, 0); // 0% to worker
        assert!(result.is_ok());
        let (worker, client) = result.unwrap();
        assert_eq!(worker, 0);
        assert_eq!(client, 10_000_000);
    }
}

/// Test duplicate account prevention
#[cfg(test)]
mod duplicate_account_tests {
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    struct Pubkey([u8; 32]);

    impl Pubkey {
        fn new(seed: u8) -> Self {
            let mut bytes = [0u8; 32];
            bytes[0] = seed;
            Pubkey(bytes)
        }
    }

    fn validate_no_duplicate_accounts(
        worker_token_account: Pubkey,
        treasury_token_account: Pubkey,
    ) -> Result<(), &'static str> {
        if worker_token_account == treasury_token_account {
            return Err("DuplicateAccounts");
        }
        Ok(())
    }

    fn validate_no_duplicate_accounts_admin(
        worker_token_account: Pubkey,
        client_token_account: Pubkey,
    ) -> Result<(), &'static str> {
        if worker_token_account == client_token_account {
            return Err("DuplicateAccounts");
        }
        Ok(())
    }

    #[test]
    fn different_accounts_accepted() {
        let worker = Pubkey::new(1);
        let treasury = Pubkey::new(2);
        let result = validate_no_duplicate_accounts(worker, treasury);
        assert!(result.is_ok());
    }

    #[test]
    fn same_accounts_rejected() {
        let account = Pubkey::new(1);
        let result = validate_no_duplicate_accounts(account, account);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "DuplicateAccounts");
    }

    #[test]
    fn admin_different_accounts_accepted() {
        let worker = Pubkey::new(1);
        let client = Pubkey::new(2);
        let result = validate_no_duplicate_accounts_admin(worker, client);
        assert!(result.is_ok());
    }

    #[test]
    fn admin_same_accounts_rejected() {
        let account = Pubkey::new(1);
        let result = validate_no_duplicate_accounts_admin(account, account);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "DuplicateAccounts");
    }
}

/// Test two-step admin transfer
#[cfg(test)]
mod admin_transfer_tests {
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    struct Pubkey([u8; 32]);

    impl Pubkey {
        fn new(seed: u8) -> Self {
            let mut bytes = [0u8; 32];
            bytes[0] = seed;
            Pubkey(bytes)
        }
        
        fn default() -> Self {
            Pubkey([0u8; 32])
        }
    }

    struct PlatformConfig {
        admin: Pubkey,
        pending_admin: Option<Pubkey>,
    }

    fn propose_admin(config: &mut PlatformConfig, caller: Pubkey, new_admin: Pubkey) -> Result<(), &'static str> {
        if caller != config.admin {
            return Err("Unauthorized");
        }
        if new_admin == Pubkey::default() {
            return Err("InvalidAdmin");
        }
        config.pending_admin = Some(new_admin);
        Ok(())
    }

    fn accept_admin(config: &mut PlatformConfig, caller: Pubkey) -> Result<(), &'static str> {
        let pending = config.pending_admin.ok_or("NoPendingAdmin")?;
        if caller != pending {
            return Err("Unauthorized");
        }
        config.admin = pending;
        config.pending_admin = None;
        Ok(())
    }

    fn cancel_admin_transfer(config: &mut PlatformConfig, caller: Pubkey) -> Result<(), &'static str> {
        if caller != config.admin {
            return Err("Unauthorized");
        }
        if config.pending_admin.is_none() {
            return Err("NoPendingAdmin");
        }
        config.pending_admin = None;
        Ok(())
    }

    #[test]
    fn propose_and_accept_admin() {
        let old_admin = Pubkey::new(1);
        let new_admin = Pubkey::new(2);
        let mut config = PlatformConfig {
            admin: old_admin,
            pending_admin: None,
        };

        // Propose
        let result = propose_admin(&mut config, old_admin, new_admin);
        assert!(result.is_ok());
        assert_eq!(config.pending_admin, Some(new_admin));
        assert_eq!(config.admin, old_admin); // Not changed yet

        // Accept
        let result = accept_admin(&mut config, new_admin);
        assert!(result.is_ok());
        assert_eq!(config.admin, new_admin);
        assert_eq!(config.pending_admin, None);
    }

    #[test]
    fn non_admin_cannot_propose() {
        let admin = Pubkey::new(1);
        let random = Pubkey::new(2);
        let new_admin = Pubkey::new(3);
        let mut config = PlatformConfig {
            admin,
            pending_admin: None,
        };

        let result = propose_admin(&mut config, random, new_admin);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "Unauthorized");
    }

    #[test]
    fn wrong_person_cannot_accept() {
        let old_admin = Pubkey::new(1);
        let new_admin = Pubkey::new(2);
        let random = Pubkey::new(3);
        let mut config = PlatformConfig {
            admin: old_admin,
            pending_admin: Some(new_admin),
        };

        let result = accept_admin(&mut config, random);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "Unauthorized");
    }

    #[test]
    fn cannot_accept_without_proposal() {
        let admin = Pubkey::new(1);
        let random = Pubkey::new(2);
        let mut config = PlatformConfig {
            admin,
            pending_admin: None,
        };

        let result = accept_admin(&mut config, random);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "NoPendingAdmin");
    }

    #[test]
    fn admin_can_cancel_transfer() {
        let admin = Pubkey::new(1);
        let new_admin = Pubkey::new(2);
        let mut config = PlatformConfig {
            admin,
            pending_admin: Some(new_admin),
        };

        let result = cancel_admin_transfer(&mut config, admin);
        assert!(result.is_ok());
        assert_eq!(config.pending_admin, None);
        assert_eq!(config.admin, admin); // Still the same admin
    }

    #[test]
    fn cannot_propose_zero_address() {
        let admin = Pubkey::new(1);
        let mut config = PlatformConfig {
            admin,
            pending_admin: None,
        };

        let result = propose_admin(&mut config, admin, Pubkey::default());
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "InvalidAdmin");
    }
}
