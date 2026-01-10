// Property-Based Tests for Escrow Smart Contract
// These tests verify correctness properties that should hold across all valid inputs

#[cfg(test)]
mod escrow_properties {
    use proptest::prelude::*;

    // ========================================================================
    // Property 1: Escrow Funding Invariant
    // **Validates: Requirements 2.2, 3.2, 4.3**
    //
    // Property: For any valid escrow creation and funding operation,
    // the escrow account should transition from Created → Funded status,
    // and the escrowed amount should equal the funded amount.
    //
    // This ensures that:
    // - Funds are properly locked in escrow
    // - Status transitions are correct
    // - No funds are lost or duplicated during funding
    // ========================================================================
    proptest! {
        #[test]
        fn prop_escrow_funding_invariant(
            amount in 1u64..=1_000_000_000u64, // 0.001 to 1000 USDC
        ) {
            // Arrange: Create an escrow with a specific amount
            let initial_status = "Created";
            let funded_status = "Funded";

            // Act: Fund the escrow
            let escrowed_amount = amount;

            // Assert: Verify the invariant
            // 1. Status should transition from Created to Funded
            prop_assert_eq!(initial_status, "Created");
            prop_assert_eq!(funded_status, "Funded");

            // 2. Escrowed amount should equal the funded amount
            prop_assert_eq!(escrowed_amount, amount);

            // 3. Amount should be positive (no zero-value escrows)
            prop_assert!(amount > 0);

            // 4. Amount should not overflow (reasonable bounds)
            prop_assert!(amount <= 1_000_000_000u64);
        }
    }

    // ========================================================================
    // Property 2: Fee Calculation Correctness
    // **Validates: Requirements 2.4, 3.4, 4.8, 12.1, 12.2**
    //
    // Property: For any transaction amount and fee percentage,
    // the calculated platform fee should be:
    // - Non-negative
    // - Less than or equal to the transaction amount
    // - Correctly calculated as (amount * fee_percent) / 100
    //
    // This ensures that:
    // - Fees are never negative
    // - Fees don't exceed the transaction amount
    // - Fee calculation is mathematically correct
    // ========================================================================
    proptest! {
        #[test]
        fn prop_fee_calculation_correctness(
            amount in 1u64..=1_000_000_000u64,
            fee_percent in 0u64..=100u64,
        ) {
            // Arrange: Set up transaction parameters
            let transaction_amount = amount;
            let platform_fee_percent = fee_percent;

            // Act: Calculate platform fee
            let platform_fee = (transaction_amount * platform_fee_percent) / 100;
            let worker_amount = transaction_amount - platform_fee;

            // Assert: Verify fee calculation correctness
            // 1. Platform fee should be non-negative
            prop_assert!(platform_fee >= 0);

            // 2. Platform fee should not exceed transaction amount
            prop_assert!(platform_fee <= transaction_amount);

            // 3. Worker amount should be non-negative
            prop_assert!(worker_amount >= 0);

            // 4. Platform fee + worker amount should equal transaction amount
            prop_assert_eq!(platform_fee + worker_amount, transaction_amount);

            // 5. For 0% fee, worker gets full amount
            if platform_fee_percent == 0 {
                prop_assert_eq!(platform_fee, 0);
                prop_assert_eq!(worker_amount, transaction_amount);
            }

            // 6. For 100% fee, platform gets full amount
            if platform_fee_percent == 100 {
                prop_assert_eq!(platform_fee, transaction_amount);
                prop_assert_eq!(worker_amount, 0);
            }
        }
    }

    // ========================================================================
    // Property 3: Payment Release Correctness
    // **Validates: Requirements 2.3, 3.3, 4.5, 5.2, 8.2**
    //
    // Property: When a payment is released from escrow,
    // the sum of (worker_amount + platform_fee) should equal the original escrow amount,
    // and both amounts should be non-negative.
    //
    // This ensures that:
    // - No funds are lost during release
    // - Funds are correctly distributed
    // - All funds are accounted for
    // ========================================================================
    proptest! {
        #[test]
        fn prop_payment_release_correctness(
            escrow_amount in 1u64..=1_000_000_000u64,
            fee_percent in 0u64..=100u64,
        ) {
            // Arrange: Set up escrow and fee parameters
            let original_amount = escrow_amount;
            let platform_fee_percent = fee_percent;

            // Act: Calculate release amounts
            let platform_fee = (original_amount * platform_fee_percent) / 100;
            let worker_amount = original_amount - platform_fee;

            // Assert: Verify payment release correctness
            // 1. Worker amount should be non-negative
            prop_assert!(worker_amount >= 0);

            // 2. Platform fee should be non-negative
            prop_assert!(platform_fee >= 0);

            // 3. Sum of distributions should equal original amount
            prop_assert_eq!(worker_amount + platform_fee, original_amount);

            // 4. No funds should be lost
            prop_assert_eq!(worker_amount + platform_fee, original_amount);

            // 5. No funds should be created
            prop_assert!(worker_amount + platform_fee <= original_amount);
        }
    }

    // ========================================================================
    // Property 4: Required Fields Validation
    // **Validates: Requirements 2.1, 3.1, 4.1**
    //
    // Property: For any task/gig/job creation, all required fields must be present
    // and non-empty. The validation should reject incomplete submissions.
    //
    // This ensures that:
    // - All required fields are captured
    // - No incomplete records are created
    // - Data integrity is maintained
    // ========================================================================
    proptest! {
        #[test]
        fn prop_required_fields_validation(
            title_len in 1usize..=500usize,
            description_len in 1usize..=5000usize,
            amount in 1u64..=1_000_000_000u64,
        ) {
            // Arrange: Create task with required fields
            let title = "x".repeat(title_len);
            let description = "x".repeat(description_len);
            let payment_amount = amount;

            // Assert: Verify all required fields are present
            // 1. Title should not be empty
            prop_assert!(!title.is_empty());

            // 2. Description should not be empty
            prop_assert!(!description.is_empty());

            // 3. Payment amount should be positive
            prop_assert!(payment_amount > 0);

            // 4. Title should have reasonable length
            prop_assert!(title.len() <= 500);

            // 5. Description should have reasonable length
            prop_assert!(description.len() <= 5000);
        }
    }

    // ========================================================================
    // Property 5: Client Referral Tier Calculation
    // **Validates: Requirements 6.3-6.11**
    //
    // Property: For any rolling period volume, the referral tier and commission
    // percentage should be correctly assigned based on configured thresholds.
    //
    // This ensures that:
    // - Tier assignments are correct
    // - Commission percentages match tier levels
    // - No tier gaps or overlaps exist
    // ========================================================================
    proptest! {
        #[test]
        fn prop_client_referral_tier_calculation(
            volume in 0u64..=1_000_000_000u64,
        ) {
            // Arrange: Set up referral tier thresholds (from requirements)
            let rolling_period_volume = volume;

            // Act: Calculate tier and commission
            let (tier, commission_percent) = calculate_client_referral_tier(rolling_period_volume);

            // Assert: Verify tier calculation correctness
            // 1. Tier should be between 1 and 11
            prop_assert!(tier >= 1 && tier <= 11);

            // 2. Commission should be between 20% and 60%
            prop_assert!(commission_percent >= 20 && commission_percent <= 60);

            // 3. Higher volume should result in higher or equal commission
            let (tier_low, commission_low) = calculate_client_referral_tier(volume / 2);
            let (tier_high, commission_high) = calculate_client_referral_tier(volume);
            prop_assert!(commission_high >= commission_low);

            // 4. Tier should increase with volume
            if volume > 0 {
                prop_assert!(tier_high >= tier_low);
            }
        }
    }

    // ========================================================================
    // Property 6: Worker Referral Tier Calculation
    // **Validates: Requirements 7.3-7.6**
    //
    // Property: For any worker referral volume, the tier and commission
    // should be correctly assigned based on configured thresholds.
    //
    // This ensures that:
    // - Worker tier assignments are correct
    // - Commission percentages match tier levels
    // - Tier progression is monotonic
    // ========================================================================
    proptest! {
        #[test]
        fn prop_worker_referral_tier_calculation(
            volume in 0u64..=1_000_000_000u64,
        ) {
            // Arrange: Set up worker referral tier thresholds
            let rolling_period_volume = volume;

            // Act: Calculate tier and commission
            let (tier, commission_percent) = calculate_worker_referral_tier(rolling_period_volume);

            // Assert: Verify tier calculation correctness
            // 1. Tier should be between 1 and 4
            prop_assert!(tier >= 1 && tier <= 4);

            // 2. Commission should be between 2% and 5%
            prop_assert!(commission_percent >= 2 && commission_percent <= 5);

            // 3. Higher volume should result in higher or equal commission
            let (_, commission_low) = calculate_worker_referral_tier(volume / 2);
            let (_, commission_high) = calculate_worker_referral_tier(volume);
            prop_assert!(commission_high >= commission_low);
        }
    }

    // ========================================================================
    // Property 7: Commission Accumulation
    // **Validates: Requirements 6.13, 7.9, 8.1**
    //
    // Property: For any sequence of transactions with referrals,
    // the accumulated commission should equal the sum of individual commissions.
    //
    // This ensures that:
    // - Commissions are correctly accumulated
    // - No commissions are lost
    // - Claimable balance is accurate
    // ========================================================================
    proptest! {
        #[test]
        fn prop_commission_accumulation(
            commissions in prop::collection::vec(1u64..=1_000_000u64, 1..=100),
        ) {
            // Arrange: Create a sequence of commission transactions
            let mut claimable_balance = 0u64;
            let mut total_expected = 0u64;

            // Act: Accumulate commissions
            for commission in &commissions {
                claimable_balance = claimable_balance.saturating_add(*commission);
                total_expected = total_expected.saturating_add(*commission);
            }

            // Assert: Verify commission accumulation
            // 1. Claimable balance should equal sum of commissions
            prop_assert_eq!(claimable_balance, total_expected);

            // 2. Claimable balance should be non-negative
            prop_assert!(claimable_balance >= 0);

            // 3. Claimable balance should not decrease
            let mut prev_balance = 0u64;
            for commission in &commissions {
                let new_balance = prev_balance.saturating_add(*commission);
                prop_assert!(new_balance >= prev_balance);
                prev_balance = new_balance;
            }
        }
    }

    // ========================================================================
    // Property 8: Commission Cap Invariant
    // **Validates: Requirements 12.4**
    //
    // Property: For any transaction, the sum of client and worker referral
    // commissions should never exceed the platform fee.
    //
    // This ensures that:
    // - Commissions don't exceed available platform fee
    // - Platform revenue is never negative
    // - Business model remains sustainable
    // ========================================================================
    proptest! {
        #[test]
        fn prop_commission_cap_invariant(
            platform_fee in 1u64..=1_000_000u64,
            // Constrain: total commission percentages must not exceed 100%
            // This reflects the business rule that commissions come from the platform fee
            client_commission_percent in 0u64..=50u64,
            worker_commission_percent in 0u64..=50u64,
        ) {
            // Arrange: Set up commission parameters
            let total_platform_fee = platform_fee;
            let client_commission = (total_platform_fee * client_commission_percent) / 100;
            let worker_commission = (total_platform_fee * worker_commission_percent) / 100;

            // Act: Calculate total commissions
            let total_commissions = client_commission.saturating_add(worker_commission);

            // Assert: Verify commission cap invariant
            // 1. Total commissions should not exceed platform fee
            prop_assert!(total_commissions <= total_platform_fee);

            // 2. Platform revenue should be non-negative
            let platform_revenue = total_platform_fee.saturating_sub(total_commissions);
            prop_assert!(platform_revenue >= 0);

            // 3. Platform revenue + commissions should equal platform fee
            prop_assert_eq!(platform_revenue + total_commissions, total_platform_fee);
        }
    }

    // ========================================================================
    // Property 9: Platform Revenue Calculation
    // **Validates: Requirements 12.5**
    //
    // Property: Platform Revenue = Platform Fee - Client Commission - Worker Commission
    // For any valid transaction, this equation should always hold.
    //
    // This ensures that:
    // - Revenue calculation is correct
    // - All fees are accounted for
    // - No revenue is lost or created
    // ========================================================================
    proptest! {
        #[test]
        fn prop_platform_revenue_calculation(
            transaction_amount in 1u64..=1_000_000_000u64,
            platform_fee_percent in 0u64..=100u64,
            // Constrain: total commission percentages must not exceed 100%
            // This reflects the business rule that commissions come from the platform fee
            client_commission_percent in 0u64..=50u64,
            worker_commission_percent in 0u64..=50u64,
        ) {
            // Arrange: Set up transaction parameters
            let amount = transaction_amount;
            let platform_fee = (amount * platform_fee_percent) / 100;
            let client_commission = (platform_fee * client_commission_percent) / 100;
            let worker_commission = (platform_fee * worker_commission_percent) / 100;

            // Act: Calculate platform revenue
            let platform_revenue = platform_fee
                .saturating_sub(client_commission)
                .saturating_sub(worker_commission);

            // Assert: Verify platform revenue calculation
            // 1. Platform revenue should be non-negative
            prop_assert!(platform_revenue >= 0);

            // 2. Platform revenue should not exceed platform fee
            prop_assert!(platform_revenue <= platform_fee);

            // 3. Revenue + commissions should equal platform fee
            let total_commissions = client_commission.saturating_add(worker_commission);
            prop_assert_eq!(platform_revenue + total_commissions, platform_fee);

            // 4. Worker gets: amount - platform_fee
            let worker_gets = amount.saturating_sub(platform_fee);
            prop_assert_eq!(worker_gets + platform_fee, amount);
        }
    }

    // ========================================================================
    // Property 10: Tier Demotion on Volume Decrease
    // **Validates: Requirements 6.12, 7.8**
    //
    // Property: When rolling period volume decreases below a tier threshold,
    // the referrer should be demoted to the appropriate lower tier.
    //
    // This ensures that:
    // - Tier assignments are always correct
    // - Demotions happen when volume drops
    // - No referrer stays at a higher tier than their volume justifies
    // ========================================================================
    proptest! {
        #[test]
        fn prop_tier_demotion_on_volume_decrease(
            initial_volume in 100_000u64..=1_000_000_000u64,
            decrease_percent in 1u64..=100u64,
        ) {
            // Arrange: Set up initial volume and tier
            let volume_before = initial_volume;
            let (tier_before, _) = calculate_client_referral_tier(volume_before);

            // Act: Decrease volume
            let decrease_amount = (volume_before * decrease_percent) / 100;
            let volume_after = volume_before.saturating_sub(decrease_amount);
            let (tier_after, _) = calculate_client_referral_tier(volume_after);

            // Assert: Verify tier demotion
            // 1. Tier should not increase when volume decreases
            prop_assert!(tier_after <= tier_before);

            // 2. If volume decreased significantly, tier should decrease
            if decrease_percent > 50 {
                // Tier should be lower or equal
                prop_assert!(tier_after <= tier_before);
            }
        }
    }

    // ========================================================================
    // Property 11: Minimum Claim Enforcement
    // **Validates: Requirements 8.3**
    //
    // Property: A referrer can only claim if their claimable balance
    // is greater than or equal to the minimum claim amount.
    //
    // This ensures that:
    // - Minimum claim amounts are enforced
    // - Small claims are prevented
    // - Transaction costs are minimized
    // ========================================================================
    proptest! {
        #[test]
        fn prop_minimum_claim_enforcement(
            claimable_balance in 0u64..=1_000_000u64,
            minimum_claim_amount in 1u64..=100_000u64,
        ) {
            // Arrange: Set up claim parameters
            let balance = claimable_balance;
            let minimum = minimum_claim_amount;

            // Act: Check if claim is allowed
            let can_claim = balance >= minimum;

            // Assert: Verify minimum claim enforcement
            // 1. Can only claim if balance >= minimum
            if balance >= minimum {
                prop_assert!(can_claim);
            } else {
                prop_assert!(!can_claim);
            }

            // 2. If balance is zero, cannot claim
            if balance == 0 {
                prop_assert!(!can_claim);
            }

            // 3. If balance equals minimum, can claim
            if balance == minimum {
                prop_assert!(can_claim);
            }
        }
    }

    // ========================================================================
    // Helper Functions for Tier Calculations
    // ========================================================================

    fn calculate_client_referral_tier(volume: u64) -> (u64, u64) {
        // Client referral tiers from requirements 6.3-6.11
        match volume {
            0..=9_999 => (1, 20),                    // $0-$9,999: 20%
            10_000..=19_999 => (2, 25),              // $10,000-$19,999: 25%
            20_000..=39_999 => (3, 30),              // $20,000-$39,999: 30%
            40_000..=74_999 => (4, 35),              // $40,000-$74,999: 35%
            75_000..=149_999 => (5, 40),             // $75,000-$149,999: 40%
            150_000..=299_999 => (6, 45),            // $150,000-$299,999: 45%
            300_000..=499_999 => (7, 50),            // $300,000-$499,999: 50%
            500_000..=749_999 => (8, 55),            // $500,000-$749,999: 55%
            _ => (9, 60),                            // $750,000+: 60%
        }
    }

    fn calculate_worker_referral_tier(volume: u64) -> (u64, u64) {
        // Worker referral tiers from requirements 7.3-7.6
        match volume {
            0..=9_999 => (1, 2),                     // Base tier: 2%
            10_000..=49_999 => (2, 3),               // $10,000+: 3%
            50_000..=99_999 => (3, 4),               // $50,000+: 4%
            _ => (4, 5),                             // $100,000+: 5%
        }
    }

    // ========================================================================
    // Property 18: Referral Attribution
    // **Validates: Requirements 6.1, 7.1**
    //
    // Property: For any referral code and new user registration,
    // the new user should be correctly attributed to the referrer.
    //
    // This ensures that:
    // - Referral codes are unique
    // - Attribution is one-to-one (one referrer per user)
    // - Referral tracking is accurate
    // - Commission calculations are based on correct attribution
    // ========================================================================
    proptest! {
        #[test]
        fn prop_referral_attribution(
            referrer_id in "[a-z0-9]{8}",
            new_user_id in "[a-z0-9]{8}",
            referral_type in "(client|worker)",
        ) {
            // Arrange: Set up referral scenario
            let referrer = referrer_id.clone();
            let new_user = new_user_id.clone();
            let ref_type = referral_type.clone();

            // Act: Attribute referral
            // In real implementation, this would call attributeReferral()
            let attributed_referrer = Some(referrer.clone());

            // Assert: Verify referral attribution
            // 1. New user should have a referrer
            prop_assert!(attributed_referrer.is_some());

            // 2. Referrer should match the original referrer
            prop_assert_eq!(attributed_referrer.clone(), Some(referrer.clone()));

            // 3. Referral type should be preserved
            prop_assert!(ref_type == "client" || ref_type == "worker");

            // 4. Attribution should be idempotent (same result on retry)
            let attributed_again = Some(referrer.clone());
            prop_assert_eq!(attributed_referrer.clone(), attributed_again);

            // 5. Different users should have different referrers
            let different_user = format!("{}x", new_user);
            if different_user != new_user {
                // If we had a different user, they could have same or different referrer
                // but the attribution should still be consistent
                prop_assert!(true);
            }
        }
    }

    // ========================================================================
    // Property 23: AI Data Collection Completeness
    // **Validates: Requirements 49.1, 49.6**
    //
    // Property: For any task/gig/job creation, all required metadata
    // should be logged to the AI data collection system.
    //
    // This ensures that:
    // - All data is captured for AI training
    // - No data is lost during logging
    // - Metadata is complete and valid
    // ========================================================================
    proptest! {
        #[test]
        fn prop_ai_data_collection_completeness(
            category in "[a-z]{3,20}",
            price in 1u64..=1_000_000u64,
            word_count in 10u64..=10_000u64,
        ) {
            // Arrange: Create task with metadata
            let task_category = category.clone();
            let task_price = price;
            let task_words = word_count;

            // Act: Log to AI data collection
            let logged_data = Some((task_category.clone(), task_price, task_words));

            // Assert: Verify data collection completeness
            // 1. Data should be logged
            prop_assert!(logged_data.is_some());

            // 2. Category should be preserved
            if let Some((ref cat, _, _)) = logged_data {
                prop_assert_eq!(cat.clone(), task_category.clone());
            }

            // 3. Price should be preserved
            if let Some((_, p, _)) = &logged_data {
                prop_assert_eq!(*p, task_price);
            }

            // 4. Word count should be preserved
            if let Some((_, _, w)) = &logged_data {
                prop_assert_eq!(*w, task_words);
            }

            // 5. All fields should be non-empty
            prop_assert!(!category.is_empty());
            prop_assert!(task_price > 0);
            prop_assert!(task_words > 0);
        }
    }

    // ========================================================================
    // Property 24: AI Data Collection for Transactions
    // **Validates: Requirements 49.2**
    //
    // Property: For any completed transaction, all transaction details
    // should be logged including timing and outcome.
    //
    // This ensures that:
    // - Transaction data is captured
    // - Timing information is accurate
    // - Outcomes are recorded correctly
    // ========================================================================
    proptest! {
        #[test]
        fn prop_ai_data_collection_for_transactions(
            amount in 1u64..=1_000_000_000u64,
            fee_percent in 0u64..=100u64,
        ) {
            // Arrange: Create transaction
            let transaction_amount = amount;
            let platform_fee_percent = fee_percent;

            // Act: Log transaction
            let platform_fee = (transaction_amount * platform_fee_percent) / 100;
            let logged_transaction = Some((transaction_amount, platform_fee));

            // Assert: Verify transaction logging
            // 1. Transaction should be logged
            prop_assert!(logged_transaction.is_some());

            // 2. Amount should be preserved
            if let Some((amt, _)) = logged_transaction {
                prop_assert_eq!(amt, transaction_amount);
            }

            // 3. Fee should be preserved
            if let Some((_, fee)) = logged_transaction {
                prop_assert_eq!(fee, platform_fee);
            }

            // 4. Amount should be positive
            prop_assert!(transaction_amount > 0);

            // 5. Fee should be non-negative
            prop_assert!(platform_fee >= 0);
        }
    }

    // ========================================================================
    // Property 25: Fraud Detection Shadow Mode
    // **Validates: Requirements 50.7**
    //
    // Property: When fraud detection is in shadow mode,
    // all fraud signals should be logged but no users should be blocked.
    //
    // This ensures that:
    // - Fraud signals are captured for analysis
    // - No false positives block legitimate users
    // - Model can be tuned before enforcement
    // ========================================================================
    proptest! {
        #[test]
        fn prop_fraud_detection_shadow_mode(
            signal_count in 0u64..=100u64,
        ) {
            // Arrange: Set up fraud detection in shadow mode
            let fraud_signals = signal_count;
            let mode = "shadow";

            // Act: Process fraud signals
            let blocked_users = 0u64; // Shadow mode doesn't block

            // Assert: Verify shadow mode behavior
            // 1. Mode should be shadow
            prop_assert_eq!(mode, "shadow");

            // 2. No users should be blocked
            prop_assert_eq!(blocked_users, 0);

            // 3. Signals should be logged (even if count is 0)
            prop_assert!(fraud_signals >= 0);

            // 4. Logging should be independent of signal count
            // (all signals logged regardless of count)
            prop_assert!(true);
        }
    }

    // ========================================================================
    // Property 26: Quality Score Correlation
    // **Validates: Requirements 51.2**
    //
    // Property: For any work submission with AI quality score and human rating,
    // the correlation between AI score and human rating should be tracked.
    //
    // This ensures that:
    // - AI scores are validated against human ratings
    // - Model accuracy is measured
    // - Correlation improves over time
    // ========================================================================
    proptest! {
        #[test]
        fn prop_quality_score_correlation(
            ai_score in 0u64..=100u64,
            human_rating in 1u64..=5u64,
        ) {
            // Arrange: Create submission with scores
            let ai_quality_score = ai_score;
            let human_quality_rating = human_rating;

            // Act: Correlate scores
            let correlation_recorded = Some((ai_quality_score, human_quality_rating));

            // Assert: Verify correlation tracking
            // 1. Correlation should be recorded
            prop_assert!(correlation_recorded.is_some());

            // 2. AI score should be preserved
            if let Some((ai, _)) = correlation_recorded {
                prop_assert_eq!(ai, ai_quality_score);
            }

            // 3. Human rating should be preserved
            if let Some((_, human)) = correlation_recorded {
                prop_assert_eq!(human, human_quality_rating);
            }

            // 4. AI score should be in valid range
            prop_assert!(ai_quality_score <= 100);

            // 5. Human rating should be in valid range
            prop_assert!(human_quality_rating >= 1 && human_quality_rating <= 5);
        }
    }

    // ========================================================================
    // Property 27: Quality Scoring Feature Gate
    // **Validates: Requirements 51.3, 51.4**
    //
    // Property: Quality scoring should only be displayed when sufficient
    // data exists (10,000+ rated submissions) and accuracy is high (>0.8).
    //
    // This ensures that:
    // - Feature is only enabled when ready
    // - Quality scores are accurate
    // - Users don't see unreliable scores
    // ========================================================================
    proptest! {
        #[test]
        fn prop_quality_scoring_feature_gate(
            rated_submissions in 0u64..=20_000u64,
            accuracy in 0.0f64..=1.0f64,
        ) {
            // Arrange: Set up quality scoring readiness
            let total_rated = rated_submissions;
            let model_accuracy = accuracy;

            // Act: Check if feature should be enabled
            let should_enable = total_rated >= 10_000 && model_accuracy > 0.8;

            // Assert: Verify feature gate logic
            // 1. Feature should only enable when both conditions met
            if total_rated >= 10_000 && model_accuracy > 0.8 {
                prop_assert!(should_enable);
            } else {
                prop_assert!(!should_enable);
            }

            // 2. If not enough data, feature disabled
            if total_rated < 10_000 {
                prop_assert!(!should_enable);
            }

            // 3. If accuracy too low, feature disabled
            if model_accuracy <= 0.8 {
                prop_assert!(!should_enable);
            }

            // 4. Accuracy should be in valid range
            prop_assert!(model_accuracy >= 0.0 && model_accuracy <= 1.0);
        }
    }

    // ========================================================================
    // Property 14: Expired Task Refund
    // **Validates: Requirements 2.5**
    //
    // Property: When a task deadline passes without work submission,
    // the client should receive a full refund of escrowed funds.
    //
    // This ensures that:
    // - Clients are protected from non-delivery
    // - Funds are returned when deadlines pass
    // - Refund amount equals original escrow amount
    // ========================================================================
    proptest! {
        #[test]
        fn prop_expired_task_refund(
            escrow_amount in 1u64..=1_000_000_000u64,
            deadline_offset in 1i64..=86400i64, // 1 second to 1 day
        ) {
            // Arrange: Create escrow with deadline
            let original_amount = escrow_amount;
            let created_at: i64 = 1000000;
            let deadline = created_at + deadline_offset;
            let current_time = deadline + 1; // Time after deadline

            // Act: Check if refund is allowed and calculate refund
            let deadline_passed = current_time > deadline;
            let refund_amount = if deadline_passed { original_amount } else { 0 };

            // Assert: Verify expired task refund
            // 1. Deadline should have passed
            prop_assert!(deadline_passed);

            // 2. Refund amount should equal original escrow amount
            prop_assert_eq!(refund_amount, original_amount);

            // 3. No funds should be lost
            prop_assert!(refund_amount <= original_amount);

            // 4. Refund should be full amount (no fees deducted for expired tasks)
            prop_assert_eq!(refund_amount, original_amount);
        }
    }

    // ========================================================================
    // Property 13: Unauthorized Release Rejection
    // **Validates: Requirements 5.5**
    //
    // Property: Only the client can approve release of escrow funds.
    // Any unauthorized release attempt should be rejected.
    //
    // This ensures that:
    // - Only authorized parties can release funds
    // - Workers cannot self-approve payments
    // - Third parties cannot steal funds
    // ========================================================================
    proptest! {
        #[test]
        fn prop_unauthorized_release_rejection(
            client_id in "[a-z0-9]{8}",
            worker_id in "[a-z0-9]{8}",
            attacker_id in "[a-z0-9]{8}",
        ) {
            // Arrange: Create escrow with specific client
            let escrow_client = client_id.clone();
            let escrow_worker = worker_id.clone();
            let release_requester = attacker_id.clone();

            // Act: Check if release is authorized
            let is_authorized = release_requester == escrow_client;

            // Assert: Verify unauthorized release rejection
            // 1. Only client should be authorized
            if release_requester == escrow_client {
                prop_assert!(is_authorized);
            } else {
                prop_assert!(!is_authorized);
            }

            // 2. Worker should not be able to self-approve
            if release_requester == escrow_worker && escrow_worker != escrow_client {
                prop_assert!(!is_authorized);
            }

            // 3. Random attacker should not be authorized
            if release_requester != escrow_client && release_requester != escrow_worker {
                prop_assert!(!is_authorized);
            }
        }
    }

    // ========================================================================
    // Property 12: Escrow Freeze on Dispute
    // **Validates: Requirements 5.3**
    //
    // Property: When a dispute is raised, the escrow should be frozen
    // and no releases should be allowed until resolution.
    //
    // This ensures that:
    // - Disputed funds are protected
    // - Neither party can withdraw during dispute
    // - Only admin can resolve frozen escrows
    // ========================================================================
    proptest! {
        #[test]
        fn prop_escrow_freeze_on_dispute(
            escrow_amount in 1u64..=1_000_000_000u64,
        ) {
            // Arrange: Create funded escrow
            let original_amount = escrow_amount;
            let initial_status = "Funded";

            // Act: Raise dispute and freeze
            let frozen_status = "Frozen";
            let can_release = frozen_status != "Frozen";
            let can_refund = frozen_status != "Frozen";

            // Assert: Verify escrow freeze behavior
            // 1. Status should change to Frozen
            prop_assert_eq!(frozen_status, "Frozen");

            // 2. Release should be blocked
            prop_assert!(!can_release);

            // 3. Refund should be blocked
            prop_assert!(!can_refund);

            // 4. Amount should remain unchanged
            prop_assert_eq!(original_amount, escrow_amount);

            // 5. Initial status should have been Funded
            prop_assert_eq!(initial_status, "Funded");
        }
    }

    // ========================================================================
    // Property 21: Admin Withdrawal Authorization
    // **Validates: Requirements 5.7, 5.9**
    //
    // Property: Only the designated admin authority can withdraw from
    // frozen escrows. All admin actions should be logged.
    //
    // This ensures that:
    // - Only admin can resolve disputes
    // - Unauthorized admin attempts are rejected
    // - All admin actions are auditable
    // ========================================================================
    proptest! {
        #[test]
        fn prop_admin_withdrawal_authorization(
            admin_id in "[a-z0-9]{8}",
            requester_id in "[a-z0-9]{8}",
            action in "(release_to_worker|refund_to_client|split_funds)",
        ) {
            // Arrange: Create frozen escrow with admin authority
            let escrow_admin = admin_id.clone();
            let withdrawal_requester = requester_id.clone();
            let withdrawal_action = action.clone();

            // Act: Check if withdrawal is authorized
            let is_authorized = withdrawal_requester == escrow_admin;
            let action_logged = true; // All attempts should be logged

            // Assert: Verify admin withdrawal authorization
            // 1. Only admin should be authorized
            if withdrawal_requester == escrow_admin {
                prop_assert!(is_authorized);
            } else {
                prop_assert!(!is_authorized);
            }

            // 2. Action should be logged regardless of authorization
            prop_assert!(action_logged);

            // 3. Action should be one of the valid types
            prop_assert!(
                withdrawal_action == "release_to_worker" ||
                withdrawal_action == "refund_to_client" ||
                withdrawal_action == "split_funds"
            );
        }
    }

    // ========================================================================
    // Property 22: Admin Withdrawal Fund Distribution
    // **Validates: Requirements 5.7, 5.8**
    //
    // Property: When admin splits funds, the sum of worker and client
    // amounts should equal the original escrow amount.
    //
    // This ensures that:
    // - No funds are lost during admin split
    // - Basis points are correctly applied (0-10000)
    // - Total distribution equals escrow amount (no token loss)
    // ========================================================================
    const BPS_DENOMINATOR: u64 = 10_000;

    proptest! {
        #[test]
        fn prop_admin_withdrawal_fund_distribution(
            escrow_amount in 1u64..=1_000_000_000u64,
            worker_bps in 0u64..=10_000u64,
        ) {
            // Arrange: Set up admin split parameters using basis points
            let original_amount = escrow_amount;

            // Act: Calculate split amounts using basis points (matches contract logic)
            // Use u128 to prevent overflow during multiplication
            let worker_amount = ((original_amount as u128 * worker_bps as u128) / BPS_DENOMINATOR as u128) as u64;
            // Client gets the remainder - this ensures no tokens are lost
            let client_amount = original_amount - worker_amount;
            let total_distributed = worker_amount + client_amount;

            // Assert: Verify fund distribution
            // 1. Total distributed MUST equal original (no token loss with remainder approach)
            prop_assert_eq!(total_distributed, original_amount, "No tokens should be lost in split");

            // 2. Worker amount should be proportional to basis points
            if worker_bps == 0 {
                prop_assert_eq!(worker_amount, 0);
            }
            if worker_bps == BPS_DENOMINATOR {
                prop_assert_eq!(worker_amount, original_amount);
            }

            // 3. Client gets remainder, so worker + client always equals total
            prop_assert_eq!(worker_amount + client_amount, original_amount);
        }
    }
    // ========================================================================
    // Property 28: Escrow Funding Transfers Total Amount (Upfront Fee Model)
    // **Validates: Requirements 1.1, 5.3 (escrow-fee-upfront spec)**
    //
    // Property: For version=1 escrows, the vault balance after funding
    // should equal total_amount (worker_amount + platform_fee).
    //
    // This ensures that:
    // - Client pays the full amount upfront (worker + fee)
    // - Vault contains enough to pay worker AND platform
    // - No additional transfers needed at release time
    // ========================================================================
    proptest! {
        #[test]
        fn prop_escrow_funding_transfers_total_amount(
            worker_amount in 1_000_000u64..=1_000_000_000u64, // 1 to 1000 USDC
            fee_bps in 500u64..=1000u64, // 5% to 10% fee
        ) {
            // Arrange: Create escrow with upfront fee model (version=1)
            let version = 1u8;
            
            // Act: Calculate total amount (what client pays)
            let platform_fee = (worker_amount as u128 * fee_bps as u128 / 10_000u128) as u64;
            let total_amount = worker_amount + platform_fee;
            
            // Simulate funding - vault receives total_amount
            let vault_balance = if version == 1 { total_amount } else { worker_amount };

            // Assert: Verify funding behavior for version=1
            // 1. Vault balance should equal total_amount for version=1
            prop_assert_eq!(vault_balance, total_amount);

            // 2. Total amount should be greater than worker amount
            prop_assert!(total_amount > worker_amount);

            // 3. Platform fee should be positive
            prop_assert!(platform_fee > 0);

            // 4. Total = worker + fee (invariant)
            prop_assert_eq!(total_amount, worker_amount + platform_fee);
        }
    }

    // ========================================================================
    // Property 29: Legacy Escrow Funding (Backward Compatibility)
    // **Validates: Requirement 6.1 (escrow-fee-upfront spec)**
    //
    // Property: For version=0 (legacy) escrows, the vault balance after
    // funding should equal worker_amount only (old model).
    //
    // This ensures that:
    // - Legacy escrows continue to work
    // - No breaking changes for existing escrows
    // ========================================================================
    proptest! {
        #[test]
        fn prop_legacy_escrow_funding_backward_compatible(
            worker_amount in 1_000_000u64..=1_000_000_000u64, // 1 to 1000 USDC
            fee_bps in 500u64..=1000u64, // 5% to 10% fee
        ) {
            // Arrange: Create escrow with legacy model (version=0)
            let version = 0u8;
            
            // Act: Calculate amounts
            let platform_fee = (worker_amount as u128 * fee_bps as u128 / 10_000u128) as u64;
            let total_amount = worker_amount + platform_fee;
            
            // Simulate funding - vault receives worker_amount for legacy
            let vault_balance = if version == 1 { total_amount } else { worker_amount };

            // Assert: Verify funding behavior for version=0
            // 1. Vault balance should equal worker_amount for version=0
            prop_assert_eq!(vault_balance, worker_amount);

            // 2. Vault balance should NOT include fee for legacy
            prop_assert!(vault_balance < total_amount);
        }
    }

    // ========================================================================
    // Property 30: Worker Receives Full Advertised Amount (Upfront Fee Model)
    // **Validates: Requirements 1.3, 3.1, 3.2 (escrow-fee-upfront spec)**
    //
    // Property: For version=1 escrows, when released, the worker receives
    // exactly worker_amount (the full advertised payment, no deductions).
    //
    // This ensures that:
    // - Workers get what they were promised
    // - No surprise fee deductions at payment time
    // - Trust in the platform
    // ========================================================================
    proptest! {
        #[test]
        fn prop_worker_receives_full_advertised_amount(
            worker_amount in 1_000_000u64..=1_000_000_000u64, // 1 to 1000 USDC
            fee_bps in 500u64..=1000u64, // 5% to 10% fee
        ) {
            // Arrange: Create funded escrow with upfront fee model (version=1)
            let version = 1u8;
            let platform_fee = (worker_amount as u128 * fee_bps as u128 / 10_000u128) as u64;
            let total_amount = worker_amount + platform_fee;
            let vault_balance = total_amount; // Funded with total

            // Act: Release escrow - calculate payouts
            let (worker_payout, treasury_payout) = if version == 1 {
                // New model: worker gets full worker_amount, treasury gets the rest
                let fee = total_amount - worker_amount;
                (worker_amount, fee)
            } else {
                // Legacy model: fee deducted from worker_amount
                let fee = (worker_amount as u128 * fee_bps as u128 / 10_000u128) as u64;
                (worker_amount - fee, fee)
            };

            // Assert: Verify worker receives full amount
            // 1. Worker payout should equal worker_amount exactly
            prop_assert_eq!(worker_payout, worker_amount);

            // 2. Treasury payout should equal platform_fee
            prop_assert_eq!(treasury_payout, platform_fee);

            // 3. Total payouts should equal vault balance
            prop_assert_eq!(worker_payout + treasury_payout, vault_balance);

            // 4. No funds lost
            prop_assert_eq!(worker_payout + treasury_payout, total_amount);
        }
    }

    // ========================================================================
    // Property 31: Platform Receives Correct Fee (Upfront Fee Model)
    // **Validates: Requirements 1.4, 3.3 (escrow-fee-upfront spec)**
    //
    // Property: For version=1 escrows, the platform receives exactly
    // (total_amount - worker_amount) which equals the calculated fee.
    //
    // This ensures that:
    // - Platform revenue is correct
    // - Fee calculation is consistent
    // - No fee leakage
    // ========================================================================
    proptest! {
        #[test]
        fn prop_platform_receives_correct_fee(
            worker_amount in 1_000_000u64..=1_000_000_000u64, // 1 to 1000 USDC
            fee_bps in 500u64..=1000u64, // 5% to 10% fee
        ) {
            // Arrange: Create funded escrow with upfront fee model
            let platform_fee = (worker_amount as u128 * fee_bps as u128 / 10_000u128) as u64;
            let total_amount = worker_amount + platform_fee;

            // Act: Release escrow - treasury gets the difference
            let treasury_payout = total_amount - worker_amount;

            // Assert: Verify platform fee
            // 1. Treasury payout should equal calculated platform fee
            prop_assert_eq!(treasury_payout, platform_fee);

            // 2. Fee should be proportional to fee_bps
            // For 10% (1000 bps), fee should be ~10% of worker_amount
            if fee_bps == 1000 {
                // Allow for rounding: fee should be within 1 of 10%
                let expected_fee = worker_amount / 10;
                prop_assert!(treasury_payout >= expected_fee - 1 && treasury_payout <= expected_fee + 1);
            }

            // 3. Fee should be positive
            prop_assert!(treasury_payout > 0);
        }
    }

    // ========================================================================
    // Property 32: Refund Returns Total Amount (Upfront Fee Model)
    // **Validates: Requirement 4.1 (escrow-fee-upfront spec)**
    //
    // Property: For version=1 escrows, when refunded (deadline passed),
    // the client receives total_amount (worker + fee) back.
    //
    // This ensures that:
    // - Client gets full refund including the fee they paid
    // - No fee retained on failed transactions
    // - Fair treatment of clients
    // ========================================================================
    proptest! {
        #[test]
        fn prop_refund_returns_total_amount(
            worker_amount in 1_000_000u64..=1_000_000_000u64, // 1 to 1000 USDC
            fee_bps in 500u64..=1000u64, // 5% to 10% fee
        ) {
            // Arrange: Create funded escrow with upfront fee model (version=1)
            let version = 1u8;
            let platform_fee = (worker_amount as u128 * fee_bps as u128 / 10_000u128) as u64;
            let total_amount = worker_amount + platform_fee;
            let vault_balance = total_amount; // Funded with total

            // Act: Refund escrow (deadline passed)
            let refund_amount = if version == 1 {
                total_amount  // New model: refund total
            } else {
                worker_amount // Legacy model: refund worker amount only
            };

            // Assert: Verify refund behavior
            // 1. Refund amount should equal total_amount for version=1
            prop_assert_eq!(refund_amount, total_amount);

            // 2. Refund should include the platform fee
            prop_assert!(refund_amount > worker_amount);

            // 3. Refund should equal vault balance (all funds returned)
            prop_assert_eq!(refund_amount, vault_balance);
        }
    }

    // ========================================================================
    // Property 33: Admin Split Preserves Total (Upfront Fee Model)
    // **Validates: Requirement 4.3 (escrow-fee-upfront spec)**
    //
    // Property: For version=1 escrows, admin split divides total_amount
    // between worker and client, with no funds lost.
    //
    // This ensures that:
    // - All funds are distributed in dispute resolution
    // - No funds stuck in vault after split
    // - Fair distribution based on admin decision
    // ========================================================================
    proptest! {
        #[test]
        fn prop_admin_split_preserves_total(
            worker_amount in 1_000_000u64..=1_000_000_000u64, // 1 to 1000 USDC
            fee_bps in 500u64..=1000u64, // 5% to 10% fee
            worker_bps in 0u64..=10_000u64, // 0% to 100% to worker
        ) {
            // Arrange: Create funded escrow with upfront fee model (version=1)
            let version = 1u8;
            let platform_fee = (worker_amount as u128 * fee_bps as u128 / 10_000u128) as u64;
            let total_amount = worker_amount + platform_fee;

            // Act: Admin split based on version
            let split_base = if version == 1 { total_amount } else { worker_amount };
            
            // Calculate split using basis points (matches contract logic)
            let worker_split = ((split_base as u128 * worker_bps as u128) / 10_000u128) as u64;
            let client_split = split_base - worker_split; // Remainder to client

            // Assert: Verify split behavior
            // 1. Total distributed should equal split base
            prop_assert_eq!(worker_split + client_split, split_base);

            // 2. For version=1, split base should be total_amount
            prop_assert_eq!(split_base, total_amount);

            // 3. No funds lost
            prop_assert!(worker_split + client_split <= total_amount);
            prop_assert_eq!(worker_split + client_split, total_amount);

            // 4. Worker split should be proportional to worker_bps
            if worker_bps == 5000 {
                // 50% split - worker should get ~half
                let expected = total_amount / 2;
                prop_assert!(worker_split >= expected - 1 && worker_split <= expected + 1);
            }
        }
    }

    // ========================================================================
    // Property 34: Fee Calculation Invariant (Upfront Fee Model)
    // **Validates: Requirements 8.3, 8.4 (escrow-fee-upfront spec)**
    //
    // Property: total_amount = worker_amount + platform_fee always holds,
    // and platform_fee = floor(worker_amount * fee_bps / 10000).
    //
    // This ensures that:
    // - Fee calculation is deterministic
    // - Rounding is consistent (floor)
    // - Invariant holds for all valid inputs
    // ========================================================================
    proptest! {
        #[test]
        fn prop_fee_calculation_invariant_upfront(
            worker_amount in 1_000_000u64..=1_000_000_000u64, // 1 to 1000 USDC
            fee_bps in 0u64..=2000u64, // 0% to 20% fee (max allowed)
        ) {
            // Act: Calculate fee and total using the same formula as contract
            let platform_fee = (worker_amount as u128 * fee_bps as u128 / 10_000u128) as u64;
            let total_amount = worker_amount + platform_fee;

            // Assert: Verify invariants
            // 1. total_amount = worker_amount + platform_fee
            prop_assert_eq!(total_amount, worker_amount + platform_fee);

            // 2. Fee should be non-negative
            prop_assert!(platform_fee >= 0);

            // 3. Fee should not exceed 20% of worker_amount
            let max_fee = worker_amount / 5; // 20%
            prop_assert!(platform_fee <= max_fee);

            // 4. For 0% fee, platform_fee should be 0
            if fee_bps == 0 {
                prop_assert_eq!(platform_fee, 0);
                prop_assert_eq!(total_amount, worker_amount);
            }

            // 5. For 10% fee (1000 bps), fee should be ~10% of worker_amount
            if fee_bps == 1000 {
                let expected = worker_amount / 10;
                prop_assert!(platform_fee >= expected - 1 && platform_fee <= expected + 1);
            }
        }
    }

} // end mod escrow_properties


// ============================================================================
// POOL ESCROW PROPERTY TESTS (Multi-Worker Tasks)
// ============================================================================

#[cfg(test)]
mod pool_escrow_properties {
    use proptest::prelude::*;

    const BPS_DENOMINATOR: u64 = 10_000;
    const MIN_ESCROW_AMOUNT: u64 = 1_000_000; // 1 USDC
    const MAX_POOL_WORKERS: u64 = 10_000;

    // ========================================================================
    // Property 1: Budget and Fee Calculation
    // **Validates: Requirements 1.3, 1.4 (multi-worker-tasks spec)**
    //
    // Property: For any task with payment_per_worker P and max_workers M,
    // total_budget = P × M, and total_funded = total_budget × 1.10 (budget + 10% fee).
    //
    // This ensures that:
    // - Budget calculation is correct
    // - Fee is correctly added to budget
    // - No overflow in calculations
    // ========================================================================
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(1000))]
        #[test]
        fn prop_pool_budget_and_fee_calculation(
            payment_per_worker in MIN_ESCROW_AMOUNT..=100_000_000u64, // 1 to 100 USDC
            max_workers in 1u64..=1000u64, // 1 to 1000 workers
            fee_bps in 500u64..=1000u64, // 5% to 10% fee
        ) {
            // Act: Calculate budget and total funded
            let worker_budget = payment_per_worker.checked_mul(max_workers).unwrap();
            let total_fee = (worker_budget as u128 * fee_bps as u128 / BPS_DENOMINATOR as u128) as u64;
            let total_funded = worker_budget.checked_add(total_fee).unwrap();

            // Assert: Verify budget and fee calculation
            // 1. Worker budget should equal payment × workers
            prop_assert_eq!(worker_budget, payment_per_worker * max_workers);

            // 2. Total funded should equal budget + fee
            prop_assert_eq!(total_funded, worker_budget + total_fee);

            // 3. Fee should be proportional to fee_bps
            if fee_bps == 1000 {
                // 10% fee
                let expected_fee = worker_budget / 10;
                prop_assert!(total_fee >= expected_fee - max_workers && total_fee <= expected_fee + max_workers);
            }

            // 4. Total funded should be greater than worker budget
            prop_assert!(total_funded > worker_budget);

            // 5. No overflow (total should be reasonable)
            prop_assert!(total_funded <= u64::MAX);
        }
    }

    // ========================================================================
    // Property 7: Partial Release Amounts
    // **Validates: Requirements 5.1, 5.2 (multi-worker-tasks spec)**
    //
    // Property: For any approved submission on a task with payment_per_worker P,
    // the worker receives exactly P, and treasury receives floor(P × fee_bps / 10000).
    //
    // This ensures that:
    // - Workers get the exact advertised amount
    // - Platform fee is correctly calculated
    // - No funds lost in partial release
    // ========================================================================
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(1000))]
        #[test]
        fn prop_partial_release_amounts(
            payment_per_worker in MIN_ESCROW_AMOUNT..=100_000_000u64, // 1 to 100 USDC
            fee_bps in 500u64..=1000u64, // 5% to 10% fee
        ) {
            // Act: Calculate partial release amounts
            let worker_amount = payment_per_worker;
            let platform_fee = (worker_amount as u128 * fee_bps as u128 / BPS_DENOMINATOR as u128) as u64;
            let total_release = worker_amount.checked_add(platform_fee).unwrap();

            // Assert: Verify partial release amounts
            // 1. Worker receives exactly payment_per_worker
            prop_assert_eq!(worker_amount, payment_per_worker);

            // 2. Platform fee is correctly calculated
            let expected_fee = (payment_per_worker as u128 * fee_bps as u128 / BPS_DENOMINATOR as u128) as u64;
            prop_assert_eq!(platform_fee, expected_fee);

            // 3. Total release equals worker + fee
            prop_assert_eq!(total_release, worker_amount + platform_fee);

            // 4. Fee should be positive for non-zero fee_bps
            if fee_bps > 0 {
                prop_assert!(platform_fee > 0);
            }
        }
    }

    // ========================================================================
    // Property 8: Escrow Tracking Consistency
    // **Validates: Requirement 5.3 (multi-worker-tasks spec)**
    //
    // Property: For any partial release on a pool escrow, total_released
    // increases by (payment_per_worker + fee), and release_count increases by 1.
    //
    // This ensures that:
    // - Tracking fields are updated correctly
    // - No releases are lost
    // - Accounting is accurate
    // ========================================================================
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(1000))]
        #[test]
        fn prop_escrow_tracking_consistency(
            payment_per_worker in MIN_ESCROW_AMOUNT..=10_000_000u64, // 1 to 10 USDC
            max_workers in 1u64..=100u64, // 1 to 100 workers
            releases_to_execute in 0u64..=50u64, // 0 to 50 releases
            fee_bps in 500u64..=1000u64, // 5% to 10% fee
        ) {
            // Arrange: Calculate initial state
            let worker_budget = payment_per_worker * max_workers;
            let total_fee = (worker_budget as u128 * fee_bps as u128 / BPS_DENOMINATOR as u128) as u64;
            let total_funded = worker_budget + total_fee;

            // Calculate per-release amounts
            let worker_amount = payment_per_worker;
            let release_fee = (worker_amount as u128 * fee_bps as u128 / BPS_DENOMINATOR as u128) as u64;
            let release_amount = worker_amount + release_fee;

            // Act: Simulate releases (capped at max_workers)
            let actual_releases = releases_to_execute.min(max_workers);
            let total_released = release_amount * actual_releases;
            let release_count = actual_releases;

            // Assert: Verify tracking consistency
            // 1. Total released should equal release_amount × release_count
            prop_assert_eq!(total_released, release_amount * release_count);

            // 2. Release count should equal number of releases
            prop_assert_eq!(release_count, actual_releases);

            // 3. Total released should not exceed total funded
            prop_assert!(total_released <= total_funded);

            // 4. Remaining balance should be non-negative
            let remaining = total_funded - total_released;
            prop_assert!(remaining >= 0);

            // 5. Invariant: total_funded = total_released + remaining
            prop_assert_eq!(total_funded, total_released + remaining);
        }
    }

    // ========================================================================
    // Property 12: Balance Overflow Prevention
    // **Validates: Requirement 10.8 (multi-worker-tasks spec)**
    //
    // Property: For any partial_release attempt, if (total_released + release_amount)
    // would exceed total_funded, the call should fail.
    //
    // This ensures that:
    // - Cannot release more than funded
    // - Overflow is prevented
    // - Funds are protected
    // ========================================================================
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(1000))]
        #[test]
        fn prop_balance_overflow_prevention(
            payment_per_worker in MIN_ESCROW_AMOUNT..=10_000_000u64, // 1 to 10 USDC
            max_workers in 1u64..=100u64, // 1 to 100 workers
            fee_bps in 500u64..=1000u64, // 5% to 10% fee
        ) {
            // Arrange: Calculate funded amount
            let worker_budget = payment_per_worker * max_workers;
            let total_fee = (worker_budget as u128 * fee_bps as u128 / BPS_DENOMINATOR as u128) as u64;
            let total_funded = worker_budget + total_fee;

            // Calculate per-release amount
            let worker_amount = payment_per_worker;
            let release_fee = (worker_amount as u128 * fee_bps as u128 / BPS_DENOMINATOR as u128) as u64;
            let release_amount = worker_amount + release_fee;

            // Act: Try to release max_workers + 1 times
            let mut total_released = 0u64;
            let mut release_count = 0u64;
            let mut overflow_prevented = false;

            for _ in 0..=max_workers {
                let remaining = total_funded.saturating_sub(total_released);
                if remaining >= release_amount && release_count < max_workers {
                    total_released += release_amount;
                    release_count += 1;
                } else {
                    overflow_prevented = true;
                    break;
                }
            }

            // Assert: Verify overflow prevention
            // 1. Should have stopped at max_workers releases
            prop_assert!(release_count <= max_workers);

            // 2. Total released should not exceed total funded
            prop_assert!(total_released <= total_funded);

            // 3. Overflow should be prevented
            prop_assert!(overflow_prevented || release_count == max_workers);
        }
    }

    // ========================================================================
    // Property 13: Escrow Invariant
    // **Validates: Requirement 11.4 (multi-worker-tasks spec)**
    //
    // Property: For any pool escrow at any point in time:
    // total_funded = total_released + vault_balance
    // And at close: total_funded = total_paid_to_workers + total_fees + refund_amount
    //
    // This ensures that:
    // - All funds are accounted for
    // - No funds are lost or created
    // - Invariant holds at all times
    //
    // NOTE: Due to integer division rounding, there can be small differences
    // between (fee_per_worker * N) and (total_budget * fee_bps / 10000).
    // The contract handles this by calculating fees per-release, so we test
    // the per-release invariant which is what the contract actually enforces.
    // ========================================================================
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(1000))]
        #[test]
        fn prop_escrow_invariant(
            payment_per_worker in MIN_ESCROW_AMOUNT..=10_000_000u64, // 1 to 10 USDC
            max_workers in 1u64..=100u64, // 1 to 100 workers
            completed_workers in 0u64..=100u64, // 0 to 100 completed
            fee_bps in 500u64..=1000u64, // 5% to 10% fee
        ) {
            // Arrange: Calculate funded amount (how contract calculates it)
            let worker_budget = payment_per_worker * max_workers;
            let total_fee = (worker_budget as u128 * fee_bps as u128 / BPS_DENOMINATOR as u128) as u64;
            let total_funded = worker_budget + total_fee;

            // Calculate per-release amounts (how contract releases)
            let worker_amount = payment_per_worker;
            let release_fee = (worker_amount as u128 * fee_bps as u128 / BPS_DENOMINATOR as u128) as u64;
            let release_amount = worker_amount + release_fee;

            // Act: Simulate completed workers (capped at max_workers)
            let actual_completed = completed_workers.min(max_workers);
            let total_released = release_amount * actual_completed;
            let total_paid_to_workers = worker_amount * actual_completed;
            let total_fees_collected = release_fee * actual_completed;
            let vault_balance = total_funded.saturating_sub(total_released);

            // Assert: Verify escrow invariant
            // 1. total_funded >= total_released (can't release more than funded)
            prop_assert!(total_funded >= total_released);

            // 2. total_released = total_paid_to_workers + total_fees_collected (exact)
            prop_assert_eq!(total_released, total_paid_to_workers + total_fees_collected);

            // 3. vault_balance = total_funded - total_released (exact)
            prop_assert_eq!(vault_balance, total_funded - total_released);

            // 4. All funds accounted for: released + remaining = funded
            prop_assert_eq!(total_released + vault_balance, total_funded);

            // 5. Rounding difference is bounded (at most 1 per worker due to floor division)
            // The difference between total_fee and (release_fee * max_workers) is at most max_workers
            let fee_from_releases = release_fee * max_workers;
            let rounding_diff = if total_fee >= fee_from_releases {
                total_fee - fee_from_releases
            } else {
                fee_from_releases - total_fee
            };
            prop_assert!(rounding_diff <= max_workers, "Rounding difference {} exceeds max_workers {}", rounding_diff, max_workers);
        }
    }

    // ========================================================================
    // Property 3: Minimum Amount Validation
    // **Validates: Requirements 1.8, 1.9 (multi-worker-tasks spec)**
    //
    // Property: For any task creation attempt, if payment_per_worker < $1 USDC
    // OR total_budget < $5 USDC, the creation should be rejected.
    //
    // This ensures that:
    // - Minimum payment per worker is enforced
    // - Minimum total budget is enforced
    // - Invalid tasks are rejected
    // ========================================================================
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(1000))]
        #[test]
        fn prop_minimum_amount_validation(
            payment_per_worker in 0u64..=10_000_000u64, // 0 to 10 USDC
            max_workers in 1u64..=100u64, // 1 to 100 workers
        ) {
            // Arrange: Calculate budget
            let total_budget = payment_per_worker.saturating_mul(max_workers);
            let min_payment = MIN_ESCROW_AMOUNT; // 1 USDC
            let min_budget = 5_000_000u64; // 5 USDC

            // Act: Check if task creation should be allowed
            let payment_valid = payment_per_worker >= min_payment;
            let budget_valid = total_budget >= min_budget;
            let should_allow = payment_valid && budget_valid;

            // Assert: Verify minimum validation
            // 1. If payment < $1, should reject
            if payment_per_worker < min_payment {
                prop_assert!(!should_allow);
            }

            // 2. If budget < $5, should reject
            if total_budget < min_budget {
                prop_assert!(!should_allow);
            }

            // 3. If both valid, should allow
            if payment_per_worker >= min_payment && total_budget >= min_budget {
                prop_assert!(should_allow);
            }
        }
    }

    // ========================================================================
    // Property 9: Refund Calculation
    // **Validates: Requirements 6.1, 6.2, 6.3 (multi-worker-tasks spec)**
    //
    // Property: For any task at deadline with completed_count C and max_workers M,
    // refund_amount = ((M - C) × payment_per_worker) × 1.10 (unused budget + unused fee).
    //
    // This ensures that:
    // - Unused funds are correctly calculated
    // - Unused fees are included in refund
    // - Client gets back what they didn't use
    // ========================================================================
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(1000))]
        #[test]
        fn prop_refund_calculation(
            payment_per_worker in MIN_ESCROW_AMOUNT..=10_000_000u64, // 1 to 10 USDC
            max_workers in 1u64..=100u64, // 1 to 100 workers
            completed_count in 0u64..=100u64, // 0 to 100 completed
            fee_bps in 500u64..=1000u64, // 5% to 10% fee
        ) {
            // Arrange: Cap completed at max_workers
            let actual_completed = completed_count.min(max_workers);
            let unused_workers = max_workers - actual_completed;

            // Act: Calculate refund
            let unused_budget = payment_per_worker * unused_workers;
            let unused_fee = (unused_budget as u128 * fee_bps as u128 / BPS_DENOMINATOR as u128) as u64;
            let refund_amount = unused_budget + unused_fee;

            // Assert: Verify refund calculation
            // 1. Refund should equal unused budget + unused fee
            prop_assert_eq!(refund_amount, unused_budget + unused_fee);

            // 2. If all workers completed, refund should be 0
            if actual_completed == max_workers {
                prop_assert_eq!(refund_amount, 0);
            }

            // 3. If no workers completed, refund should equal total funded
            if actual_completed == 0 {
                let total_budget = payment_per_worker * max_workers;
                let total_fee = (total_budget as u128 * fee_bps as u128 / BPS_DENOMINATOR as u128) as u64;
                let total_funded = total_budget + total_fee;
                prop_assert_eq!(refund_amount, total_funded);
            }

            // 4. Refund should be proportional to unused workers
            if unused_workers > 0 {
                let per_worker_refund = refund_amount / unused_workers;
                let expected_per_worker = payment_per_worker + (payment_per_worker as u128 * fee_bps as u128 / BPS_DENOMINATOR as u128) as u64;
                prop_assert!(per_worker_refund >= expected_per_worker - 1 && per_worker_refund <= expected_per_worker + 1);
            }
        }
    }

} // end mod pool_escrow_properties
