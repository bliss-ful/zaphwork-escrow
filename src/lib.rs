use anchor_lang::prelude::*;
use anchor_spl::token::{self, CloseAccount, Mint, Token, TokenAccount, Transfer};
use std::fmt;

declare_id!("3iKABSF5zoQjGPykxUQNxbm7eQADqs8DreuGupggc679");

// ============================================================================
// CONSTANTS
// ============================================================================

/// Default fee for tasks/gigs/projects (10% = 1000 basis points)
/// HARDCODED: Cannot be changed by admin - ensures trustless operation
pub const DEFAULT_FEE_BPS: u64 = 1000;

/// Employment fee for jobs (5% = 500 basis points)
/// HARDCODED: Cannot be changed by admin - ensures trustless operation
pub const EMPLOYMENT_FEE_BPS: u64 = 500;

/// Minimum escrow amount (1 USDC = 1_000_000 with 6 decimals)
pub const MIN_ESCROW_AMOUNT: u64 = 1_000_000;

/// Basis points denominator
pub const BPS_DENOMINATOR: u64 = 10_000;

/// Escrow seed prefix
pub const ESCROW_SEED: &[u8] = b"escrow";

/// Pool escrow seed prefix (for multi-worker tasks)
pub const POOL_ESCROW_SEED: &[u8] = b"pool_escrow";

/// Pool vault seed prefix
pub const POOL_VAULT_SEED: &[u8] = b"pool_vault";

/// Escrow vault seed prefix  
pub const VAULT_SEED: &[u8] = b"vault";

/// Maximum number of workers for a pool escrow
pub const MAX_POOL_WORKERS: u64 = 10_000;

/// Expected admin pubkey - This is the deployer wallet that can initialize config
/// This prevents front-running attacks on initialize_config
pub const EXPECTED_ADMIN: &str = "HDz6adAXLsReVUigJbkFn5rNkJdAyyECqfxyAtypa28S";

/// Maximum escrow duration (1 year in seconds)
/// Prevents unrealistic deadlines
pub const MAX_ESCROW_DURATION: i64 = 365 * 24 * 60 * 60;

#[program]
pub mod zaphwork {
    use super::*;

    /// Initialize the platform config (one-time setup by deployer)
    /// NOTE: Only the EXPECTED_ADMIN can call this to prevent front-running
    /// NOTE: Fees are HARDCODED as constants - cannot be changed after deployment
    pub fn initialize_config(
        ctx: Context<InitializeConfig>,
        platform_authority: Pubkey,  // Backend hot wallet for partial_release
    ) -> Result<()> {
        // Prevent front-running by verifying expected admin
        let expected_admin = EXPECTED_ADMIN.parse::<Pubkey>().map_err(|_| EscrowError::InvalidAdmin)?;
        require!(ctx.accounts.admin.key() == expected_admin, EscrowError::InvalidAdmin);
        
        // Validate platform_authority is not zero address
        require!(platform_authority != Pubkey::default(), EscrowError::InvalidPlatformAuthority);
        
        let config = &mut ctx.accounts.config;
        config.admin = ctx.accounts.admin.key();
        config.treasury = ctx.accounts.treasury.key();
        config.platform_authority = platform_authority;
        config.paused = false;
        config.pending_admin = None;
        config.bump = ctx.bumps.config;

        emit!(ConfigInitialized {
            admin: config.admin,
            treasury: config.treasury,
            platform_authority,
        });

        Ok(())
    }

    /// Update platform config (admin only)
    /// NOTE: Fees are HARDCODED - only treasury, platform_authority, and paused can be updated
    pub fn update_config(
        ctx: Context<UpdateConfig>,
        new_treasury: Option<Pubkey>,
        new_platform_authority: Option<Pubkey>,
        paused: Option<bool>,
    ) -> Result<()> {
        let config = &mut ctx.accounts.config;

        if let Some(treasury) = new_treasury {
            config.treasury = treasury;
        }

        if let Some(authority) = new_platform_authority {
            require!(authority != Pubkey::default(), EscrowError::InvalidPlatformAuthority);
            config.platform_authority = authority;
        }

        if let Some(is_paused) = paused {
            config.paused = is_paused;
        }

        emit!(ConfigUpdated {
            admin: config.admin,
            treasury: config.treasury,
            platform_authority: config.platform_authority,
            paused: config.paused,
        });

        Ok(())
    }

    /// Propose a new admin (two-step transfer for safety)
    /// The new admin must call accept_admin to complete the transfer
    pub fn propose_admin(ctx: Context<ProposeAdmin>, new_admin: Pubkey) -> Result<()> {
        let config = &mut ctx.accounts.config;
        
        // Cannot propose zero address
        require!(new_admin != Pubkey::default(), EscrowError::InvalidAdmin);
        
        config.pending_admin = Some(new_admin);

        emit!(AdminProposed {
            current_admin: config.admin,
            proposed_admin: new_admin,
            proposed_at: Clock::get()?.unix_timestamp,
        });

        Ok(())
    }

    /// Accept admin role (must be called by the pending admin)
    pub fn accept_admin(ctx: Context<AcceptAdmin>) -> Result<()> {
        let config = &mut ctx.accounts.config;
        
        let pending = config.pending_admin.ok_or(EscrowError::NoPendingAdmin)?;
        require!(ctx.accounts.new_admin.key() == pending, EscrowError::Unauthorized);

        let old_admin = config.admin;
        config.admin = pending;
        config.pending_admin = None;

        emit!(AdminTransferred {
            old_admin,
            new_admin: config.admin,
            transferred_at: Clock::get()?.unix_timestamp,
        });

        Ok(())
    }

    /// Cancel a pending admin transfer (current admin only)
    pub fn cancel_admin_transfer(ctx: Context<UpdateConfig>) -> Result<()> {
        let config = &mut ctx.accounts.config;
        
        require!(config.pending_admin.is_some(), EscrowError::NoPendingAdmin);
        
        let cancelled_admin = config.pending_admin.take();

        emit!(AdminTransferCancelled {
            admin: config.admin,
            cancelled_pending: cancelled_admin.unwrap(),
            cancelled_at: Clock::get()?.unix_timestamp,
        });

        Ok(())
    }

    /// Initialize the escrow account as a PDA
    /// Now calculates total_amount = worker_amount + platform_fee upfront
    /// Validates worker pubkey is on ed25519 curve to prevent invalid addresses
    pub fn create_escrow(
        ctx: Context<CreateEscrow>,
        escrow_id: u64,
        worker_amount: u64,  // Renamed from 'amount' - this is what worker receives
        escrow_type: EscrowType,
        deadline: Option<i64>,
    ) -> Result<()> {
        let config = &ctx.accounts.config;
        require!(!config.paused, EscrowError::PlatformPaused);
        require!(worker_amount >= MIN_ESCROW_AMOUNT, EscrowError::AmountTooSmall);

        // Validate worker pubkey is on the ed25519 curve
        require!(
            ctx.accounts.worker.key().is_on_curve(),
            EscrowError::InvalidWorkerAddress
        );

        // Validate deadline is in the future and not too far
        if let Some(dl) = deadline {
            let current_time = Clock::get()?.unix_timestamp;
            require!(dl > current_time, EscrowError::DeadlineInPast);
            // Add maximum deadline limit (1 year)
            let max_deadline = current_time.checked_add(MAX_ESCROW_DURATION).ok_or(EscrowError::Overflow)?;
            require!(dl <= max_deadline, EscrowError::DeadlineTooFar);
        }

        // Select fee based on escrow type - HARDCODED constants
        let fee_bps = match escrow_type {
            EscrowType::Task => DEFAULT_FEE_BPS,        // 10% for tasks/gigs/projects
            EscrowType::Employment => EMPLOYMENT_FEE_BPS, // 5% for jobs
        };

        // Calculate platform fee and total amount (upfront fee model)
        let platform_fee = calculate_fee(worker_amount, fee_bps)?;
        let total_amount = worker_amount.checked_add(platform_fee).ok_or(EscrowError::Overflow)?;

        let escrow = &mut ctx.accounts.escrow;
        escrow.escrow_id = escrow_id;
        escrow.client = ctx.accounts.client.key();
        escrow.worker = ctx.accounts.worker.key();
        escrow.mint = ctx.accounts.mint.key();
        escrow.vault = ctx.accounts.vault.key();
        escrow.worker_amount = worker_amount;  // What worker receives
        escrow.total_amount = total_amount;    // What client pays (worker + fee)
        escrow.platform_fee_bps = fee_bps;
        escrow.escrow_type = escrow_type;
        escrow.status = EscrowStatus::Created;
        escrow.created_at = Clock::get()?.unix_timestamp;
        escrow.deadline = deadline;
        escrow.funded_at = None;
        escrow.released_at = None;
        escrow.refunded_at = None;
        escrow.frozen_at = None;
        escrow.bump = ctx.bumps.escrow;
        escrow.vault_bump = ctx.bumps.vault;
        escrow.version = 1;  // New upfront fee model

        emit!(EscrowCreated {
            escrow: escrow.key(),
            escrow_id,
            client: escrow.client,
            worker: escrow.worker,
            mint: escrow.mint,
            amount: worker_amount,  // Keep event field name for backward compatibility
            escrow_type,
            deadline,
        });

        Ok(())
    }

    /// Fund the escrow with tokens
    /// Transfers total_amount (worker_amount + platform_fee) from client to vault
    pub fn fund_escrow(ctx: Context<FundEscrow>) -> Result<()> {
        let escrow = &mut ctx.accounts.escrow;
        require!(escrow.status == EscrowStatus::Created, EscrowError::InvalidStatus);
        require!(ctx.accounts.client.key() == escrow.client, EscrowError::Unauthorized);

        let cpi_accounts = Transfer {
            from: ctx.accounts.client_token_account.to_account_info(),
            to: ctx.accounts.vault.to_account_info(),
            authority: ctx.accounts.client.to_account_info(),
        };
        let cpi_ctx = CpiContext::new(ctx.accounts.token_program.to_account_info(), cpi_accounts);
        token::transfer(cpi_ctx, escrow.total_amount)?;

        escrow.status = EscrowStatus::Funded;
        escrow.funded_at = Some(Clock::get()?.unix_timestamp);

        emit!(EscrowFunded {
            escrow: escrow.key(),
            amount: escrow.total_amount,
            funded_at: escrow.funded_at.unwrap(),
        });

        Ok(())
    }

    /// Release escrow to worker (client approves)
    /// Worker gets worker_amount, treasury gets platform fee (total_amount - worker_amount)
    pub fn release_escrow(ctx: Context<ReleaseEscrow>) -> Result<()> {
        let escrow = &mut ctx.accounts.escrow;
        require!(escrow.status == EscrowStatus::Funded, EscrowError::InvalidStatus);
        require!(ctx.accounts.client.key() == escrow.client, EscrowError::Unauthorized);

        // Worker gets full worker_amount, fee is the difference
        let worker_payout = escrow.worker_amount;
        let platform_fee = escrow.total_amount.checked_sub(escrow.worker_amount).ok_or(EscrowError::Overflow)?;

        let escrow_id_bytes = escrow.escrow_id.to_le_bytes();
        let seeds = &[ESCROW_SEED, escrow.client.as_ref(), escrow.worker.as_ref(), &escrow_id_bytes, &[escrow.bump]];
        let signer_seeds = &[&seeds[..]];

        if worker_payout > 0 {
            let cpi_accounts = Transfer {
                from: ctx.accounts.vault.to_account_info(),
                to: ctx.accounts.worker_token_account.to_account_info(),
                authority: escrow.to_account_info(),
            };
            let cpi_ctx = CpiContext::new_with_signer(ctx.accounts.token_program.to_account_info(), cpi_accounts, signer_seeds);
            token::transfer(cpi_ctx, worker_payout)?;
        }

        if platform_fee > 0 {
            let cpi_accounts = Transfer {
                from: ctx.accounts.vault.to_account_info(),
                to: ctx.accounts.treasury_token_account.to_account_info(),
                authority: escrow.to_account_info(),
            };
            let cpi_ctx = CpiContext::new_with_signer(ctx.accounts.token_program.to_account_info(), cpi_accounts, signer_seeds);
            token::transfer(cpi_ctx, platform_fee)?;
        }

        escrow.status = EscrowStatus::Released;
        escrow.released_at = Some(Clock::get()?.unix_timestamp);

        emit!(EscrowReleased { escrow: escrow.key(), worker_amount: worker_payout, platform_fee, released_at: escrow.released_at.unwrap() });
        Ok(())
    }

    /// Freeze escrow on dispute (client, worker, or admin can call)
    pub fn freeze_escrow(ctx: Context<FreezeEscrow>) -> Result<()> {
        let escrow = &mut ctx.accounts.escrow;
        require!(escrow.status == EscrowStatus::Funded, EscrowError::InvalidStatus);
        
        let caller = ctx.accounts.caller.key();
        require!(
            caller == escrow.client || caller == escrow.worker || caller == ctx.accounts.config.admin,
            EscrowError::Unauthorized
        );

        escrow.status = EscrowStatus::Frozen;
        escrow.frozen_at = Some(Clock::get()?.unix_timestamp);

        emit!(EscrowFrozen { escrow: escrow.key(), frozen_by: caller, frozen_at: escrow.frozen_at.unwrap() });
        Ok(())
    }

    /// Admin release to worker (dispute resolution)
    /// Worker wins dispute - gets worker_amount, platform gets fee.
    /// Platform retains fee when worker is awarded the funds.
    pub fn admin_release_to_worker(ctx: Context<AdminActionCtx>) -> Result<()> {
        let escrow = &mut ctx.accounts.escrow;
        let config = &ctx.accounts.config;
        
        require!(escrow.status == EscrowStatus::Frozen, EscrowError::InvalidStatus);
        require!(ctx.accounts.admin.key() == config.admin, EscrowError::Unauthorized);
        // Verify escrow was actually funded before admin action
        require!(escrow.funded_at.is_some(), EscrowError::NotFunded);

        // Worker wins - gets worker_amount, platform gets fee
        let worker_payout = escrow.worker_amount;
        let platform_fee = escrow.total_amount.checked_sub(escrow.worker_amount).ok_or(EscrowError::Overflow)?;

        let escrow_id_bytes = escrow.escrow_id.to_le_bytes();
        let seeds = &[ESCROW_SEED, escrow.client.as_ref(), escrow.worker.as_ref(), &escrow_id_bytes, &[escrow.bump]];
        let signer_seeds = &[&seeds[..]];

        // Transfer to worker
        let cpi_accounts = Transfer {
            from: ctx.accounts.vault.to_account_info(),
            to: ctx.accounts.worker_token_account.to_account_info(),
            authority: escrow.to_account_info(),
        };
        let cpi_ctx = CpiContext::new_with_signer(ctx.accounts.token_program.to_account_info(), cpi_accounts, signer_seeds);
        token::transfer(cpi_ctx, worker_payout)?;

        // Transfer fee to treasury
        if platform_fee > 0 {
            let cpi_accounts = Transfer {
                from: ctx.accounts.vault.to_account_info(),
                to: ctx.accounts.treasury_token_account.to_account_info(),
                authority: escrow.to_account_info(),
            };
            let cpi_ctx = CpiContext::new_with_signer(ctx.accounts.token_program.to_account_info(), cpi_accounts, signer_seeds);
            token::transfer(cpi_ctx, platform_fee)?;
        }

        escrow.status = EscrowStatus::Released;
        escrow.released_at = Some(Clock::get()?.unix_timestamp);

        emit!(AdminActionEvent { escrow: escrow.key(), action: "release_to_worker".to_string(), amount: worker_payout, admin: config.admin, timestamp: escrow.released_at.unwrap() });
        Ok(())
    }

    /// Admin refund to client (dispute resolution)
    /// Client wins dispute - gets FULL refund including platform fee.
    /// Platform absorbs the cost when client is not at fault.
    /// For version=1: client receives total_amount (worker_amount + fee)
    pub fn admin_refund_to_client(ctx: Context<AdminRefundCtx>) -> Result<()> {
        let escrow = &mut ctx.accounts.escrow;
        let config = &ctx.accounts.config;
        
        require!(escrow.status == EscrowStatus::Frozen, EscrowError::InvalidStatus);
        require!(ctx.accounts.admin.key() == config.admin, EscrowError::Unauthorized);
        // Verify escrow was actually funded before admin action
        require!(escrow.funded_at.is_some(), EscrowError::NotFunded);

        // Client wins dispute - full refund (no fee taken)
        let client_refund = escrow.total_amount;

        let escrow_id_bytes = escrow.escrow_id.to_le_bytes();
        let seeds = &[ESCROW_SEED, escrow.client.as_ref(), escrow.worker.as_ref(), &escrow_id_bytes, &[escrow.bump]];
        let signer_seeds = &[&seeds[..]];

        // Transfer full amount to client
        let cpi_accounts = Transfer {
            from: ctx.accounts.vault.to_account_info(),
            to: ctx.accounts.client_token_account.to_account_info(),
            authority: escrow.to_account_info(),
        };
        let cpi_ctx = CpiContext::new_with_signer(ctx.accounts.token_program.to_account_info(), cpi_accounts, signer_seeds);
        token::transfer(cpi_ctx, client_refund)?;

        escrow.status = EscrowStatus::Refunded;
        escrow.refunded_at = Some(Clock::get()?.unix_timestamp);

        emit!(AdminActionEvent { escrow: escrow.key(), action: "refund_to_client".to_string(), amount: client_refund, admin: config.admin, timestamp: escrow.refunded_at.unwrap() });
        Ok(())
    }

    /// Refund escrow to client (deadline passed)
    /// Refunds total_amount (worker_amount + platform_fee) to client
    pub fn refund_escrow(ctx: Context<RefundEscrow>) -> Result<()> {
        let escrow = &mut ctx.accounts.escrow;
        
        require!(escrow.status == EscrowStatus::Funded, EscrowError::InvalidStatus);
        require!(ctx.accounts.client.key() == escrow.client, EscrowError::Unauthorized);

        let deadline = escrow.deadline.ok_or(EscrowError::NoDeadlineSet)?;
        let current_time = Clock::get()?.unix_timestamp;
        require!(current_time > deadline, EscrowError::DeadlineNotPassed);

        let escrow_id_bytes = escrow.escrow_id.to_le_bytes();
        let seeds = &[ESCROW_SEED, escrow.client.as_ref(), escrow.worker.as_ref(), &escrow_id_bytes, &[escrow.bump]];
        let signer_seeds = &[&seeds[..]];

        let cpi_accounts = Transfer {
            from: ctx.accounts.vault.to_account_info(),
            to: ctx.accounts.client_token_account.to_account_info(),
            authority: escrow.to_account_info(),
        };
        let cpi_ctx = CpiContext::new_with_signer(ctx.accounts.token_program.to_account_info(), cpi_accounts, signer_seeds);
        token::transfer(cpi_ctx, escrow.total_amount)?;

        escrow.status = EscrowStatus::Refunded;
        escrow.refunded_at = Some(Clock::get()?.unix_timestamp);

        emit!(EscrowRefunded { escrow: escrow.key(), client: escrow.client, amount: escrow.total_amount, refunded_at: escrow.refunded_at.unwrap(), reason: "deadline_passed".to_string() });
        Ok(())
    }

    /// Cancel unfunded escrow (client only)
    /// Closes both the escrow account and the vault token account, returning rent to client
    pub fn cancel_escrow(ctx: Context<CancelEscrow>) -> Result<()> {
        let escrow = &ctx.accounts.escrow;
        
        require!(escrow.status == EscrowStatus::Created, EscrowError::InvalidStatus);
        require!(ctx.accounts.client.key() == escrow.client, EscrowError::Unauthorized);
        
        // Verify vault is empty before closing (should always be true for Created status)
        require!(ctx.accounts.vault.amount == 0, EscrowError::VaultNotEmpty);

        // Close the vault token account via CPI to Token Program
        // The escrow PDA is the authority of the vault, so it must sign
        let escrow_id_bytes = escrow.escrow_id.to_le_bytes();
        let seeds = &[
            ESCROW_SEED,
            escrow.client.as_ref(),
            escrow.worker.as_ref(),
            &escrow_id_bytes,
            &[escrow.bump],
        ];
        let signer_seeds = &[&seeds[..]];

        let cpi_accounts = CloseAccount {
            account: ctx.accounts.vault.to_account_info(),
            destination: ctx.accounts.client.to_account_info(),
            authority: ctx.accounts.escrow.to_account_info(),
        };
        let cpi_ctx = CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            cpi_accounts,
            signer_seeds,
        );
        token::close_account(cpi_ctx)?;

        emit!(EscrowCancelled { escrow: escrow.key(), client: escrow.client, cancelled_at: Clock::get()?.unix_timestamp });
        Ok(())
    }

    /// Admin split funds between client and worker (dispute resolution)
    /// Platform ALWAYS retains its fee during dispute resolution.
    /// This covers the cost of adjudication regardless of outcome.
    /// NOTE: Uses basis points (0-10000) for precision. 5000 = 50% to worker.
    /// Any remainder from integer division goes to the client (who funded the escrow).
    /// Fee = (total_amount - worker_amount), split worker_amount between parties
    pub fn admin_split_funds(ctx: Context<AdminSplit>, worker_bps: u64) -> Result<()> {
        let escrow = &mut ctx.accounts.escrow;
        let config = &ctx.accounts.config;
        
        require!(escrow.status == EscrowStatus::Frozen, EscrowError::InvalidStatus);
        require!(ctx.accounts.admin.key() == config.admin, EscrowError::Unauthorized);
        // Verify escrow was actually funded before admin action
        require!(escrow.funded_at.is_some(), EscrowError::NotFunded);
        require!(worker_bps <= BPS_DENOMINATOR, EscrowError::InvalidPercentage);

        // Platform ALWAYS keeps fee - calculate amounts
        let platform_fee = escrow.total_amount.checked_sub(escrow.worker_amount).ok_or(EscrowError::Overflow)?;
        let split_base = escrow.worker_amount;

        // Calculate worker amount using basis points (0-10000) from split_base (after fee)
        // Use u128 to prevent overflow during multiplication
        let worker_amount = ((split_base as u128)
            .checked_mul(worker_bps as u128)
            .ok_or(EscrowError::Overflow)?
            .checked_div(BPS_DENOMINATOR as u128)
            .ok_or(EscrowError::Overflow)?) as u64;
        
        // Client gets the remainder of split_base - this ensures no tokens are lost
        let client_amount = split_base.checked_sub(worker_amount).ok_or(EscrowError::Overflow)?;

        let escrow_id_bytes = escrow.escrow_id.to_le_bytes();
        let seeds = &[ESCROW_SEED, escrow.client.as_ref(), escrow.worker.as_ref(), &escrow_id_bytes, &[escrow.bump]];
        let signer_seeds = &[&seeds[..]];

        // Transfer fee to treasury FIRST
        if platform_fee > 0 {
            let cpi_accounts = Transfer { from: ctx.accounts.vault.to_account_info(), to: ctx.accounts.treasury_token_account.to_account_info(), authority: escrow.to_account_info() };
            let cpi_ctx = CpiContext::new_with_signer(ctx.accounts.token_program.to_account_info(), cpi_accounts, signer_seeds);
            token::transfer(cpi_ctx, platform_fee)?;
        }

        // Transfer to worker
        if worker_amount > 0 {
            let cpi_accounts = Transfer { from: ctx.accounts.vault.to_account_info(), to: ctx.accounts.worker_token_account.to_account_info(), authority: escrow.to_account_info() };
            let cpi_ctx = CpiContext::new_with_signer(ctx.accounts.token_program.to_account_info(), cpi_accounts, signer_seeds);
            token::transfer(cpi_ctx, worker_amount)?;
        }

        // Transfer to client
        if client_amount > 0 {
            let cpi_accounts = Transfer { from: ctx.accounts.vault.to_account_info(), to: ctx.accounts.client_token_account.to_account_info(), authority: escrow.to_account_info() };
            let cpi_ctx = CpiContext::new_with_signer(ctx.accounts.token_program.to_account_info(), cpi_accounts, signer_seeds);
            token::transfer(cpi_ctx, client_amount)?;
        }

        escrow.status = EscrowStatus::Released;
        escrow.released_at = Some(Clock::get()?.unix_timestamp);

        // Convert worker_bps to percentage for event (for backward compatibility)
        let worker_percentage = worker_bps.checked_div(100).unwrap_or(0);
        emit!(AdminSplitFundsEvent { escrow: escrow.key(), worker_amount, client_amount, worker_percentage, admin: config.admin, timestamp: escrow.released_at.unwrap() });
        Ok(())
    }

    /// Close completed escrow and reclaim rent (client only)
    /// Closes both the escrow account and the vault token account, returning rent to client
    pub fn close_escrow(ctx: Context<CloseEscrow>) -> Result<()> {
        let escrow = &ctx.accounts.escrow;
        require!(escrow.status == EscrowStatus::Released || escrow.status == EscrowStatus::Refunded, EscrowError::InvalidStatus);
        
        // Verify vault is empty before closing (should always be true after release/refund)
        require!(ctx.accounts.vault.amount == 0, EscrowError::VaultNotEmpty);

        // Close the vault token account via CPI to Token Program
        // The escrow PDA is the authority of the vault, so it must sign
        let escrow_id_bytes = escrow.escrow_id.to_le_bytes();
        let seeds = &[
            ESCROW_SEED,
            escrow.client.as_ref(),
            escrow.worker.as_ref(),
            &escrow_id_bytes,
            &[escrow.bump],
        ];
        let signer_seeds = &[&seeds[..]];

        let cpi_accounts = CloseAccount {
            account: ctx.accounts.vault.to_account_info(),
            destination: ctx.accounts.client.to_account_info(),
            authority: ctx.accounts.escrow.to_account_info(),
        };
        let cpi_ctx = CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            cpi_accounts,
            signer_seeds,
        );
        token::close_account(cpi_ctx)?;
        
        emit!(EscrowClosed { escrow: escrow.key(), closed_at: Clock::get()?.unix_timestamp });
        Ok(())
    }

    // ========================================================================
    // POOL ESCROW INSTRUCTIONS (Multi-Worker Tasks)
    // ========================================================================

    /// Create a pool escrow for multi-worker tasks
    /// This allows a client to fund a task that multiple workers can complete
    /// Uses HARDCODED fee constants for trustless operation
    pub fn create_pool_escrow(
        ctx: Context<CreatePoolEscrow>,
        escrow_id: u64,
        payment_per_worker: u64,
        max_releases: u64,
        deadline: Option<i64>,
    ) -> Result<()> {
        let config = &ctx.accounts.config;
        require!(!config.paused, EscrowError::PlatformPaused);
        require!(payment_per_worker >= MIN_ESCROW_AMOUNT, EscrowError::AmountTooSmall);
        require!(max_releases >= 1 && max_releases <= MAX_POOL_WORKERS, EscrowError::InvalidMaxReleases);

        // Validate deadline if provided
        if let Some(dl) = deadline {
            let current_time = Clock::get()?.unix_timestamp;
            require!(dl > current_time, EscrowError::DeadlineInPast);
            let max_deadline = current_time.checked_add(MAX_ESCROW_DURATION).ok_or(EscrowError::Overflow)?;
            require!(dl <= max_deadline, EscrowError::DeadlineTooFar);
        }

        // Calculate total budget and fees using HARDCODED constant
        let worker_budget = payment_per_worker
            .checked_mul(max_releases)
            .ok_or(EscrowError::Overflow)?;
        let total_fee = calculate_fee(worker_budget, DEFAULT_FEE_BPS)?;
        let total_funded = worker_budget
            .checked_add(total_fee)
            .ok_or(EscrowError::Overflow)?;

        let pool_escrow = &mut ctx.accounts.pool_escrow;
        pool_escrow.escrow_id = escrow_id;
        pool_escrow.client = ctx.accounts.client.key();
        pool_escrow.mint = ctx.accounts.mint.key();
        pool_escrow.vault = ctx.accounts.vault.key();
        pool_escrow.payment_per_worker = payment_per_worker;
        pool_escrow.max_releases = max_releases;
        pool_escrow.total_funded = total_funded;
        pool_escrow.total_released = 0;
        pool_escrow.release_count = 0;
        pool_escrow.platform_fee_bps = DEFAULT_FEE_BPS;  // Store for reference
        pool_escrow.status = PoolEscrowStatus::Created;
        pool_escrow.created_at = Clock::get()?.unix_timestamp;
        pool_escrow.funded_at = None;
        pool_escrow.closed_at = None;
        pool_escrow.deadline = deadline;
        pool_escrow.bump = ctx.bumps.pool_escrow;
        pool_escrow.vault_bump = ctx.bumps.vault;

        emit!(PoolEscrowCreated {
            escrow: pool_escrow.key(),
            escrow_id,
            client: pool_escrow.client,
            mint: pool_escrow.mint,
            payment_per_worker,
            max_releases,
            total_funded,
            deadline,
        });

        Ok(())
    }

    /// Fund the pool escrow with tokens
    pub fn fund_pool_escrow(ctx: Context<FundPoolEscrow>) -> Result<()> {
        let pool_escrow = &mut ctx.accounts.pool_escrow;
        require!(pool_escrow.status == PoolEscrowStatus::Created, EscrowError::InvalidStatus);
        require!(ctx.accounts.client.key() == pool_escrow.client, EscrowError::Unauthorized);

        // Transfer total_funded from client to vault
        let cpi_accounts = Transfer {
            from: ctx.accounts.client_token_account.to_account_info(),
            to: ctx.accounts.vault.to_account_info(),
            authority: ctx.accounts.client.to_account_info(),
        };
        let cpi_ctx = CpiContext::new(ctx.accounts.token_program.to_account_info(), cpi_accounts);
        token::transfer(cpi_ctx, pool_escrow.total_funded)?;

        pool_escrow.status = PoolEscrowStatus::Funded;
        pool_escrow.funded_at = Some(Clock::get()?.unix_timestamp);

        emit!(PoolEscrowFunded {
            escrow: pool_escrow.key(),
            amount: pool_escrow.total_funded,
            funded_at: pool_escrow.funded_at.unwrap(),
        });

        Ok(())
    }

    /// Release payment to a single worker from the pool escrow
    /// Called by the platform backend when a worker's submission is approved
    pub fn partial_release(ctx: Context<PartialRelease>) -> Result<()> {
        let pool_escrow = &mut ctx.accounts.pool_escrow;
        let config = &ctx.accounts.config;

        // Verify escrow is funded or active
        require!(
            pool_escrow.status == PoolEscrowStatus::Funded || 
            pool_escrow.status == PoolEscrowStatus::Active,
            EscrowError::InvalidStatus
        );

        // Verify deadline hasn't passed (if set)
        if let Some(dl) = pool_escrow.deadline {
            require!(Clock::get()?.unix_timestamp <= dl, EscrowError::DeadlinePassed);
        }

        // Verify we haven't exceeded max releases
        require!(pool_escrow.release_count < pool_escrow.max_releases, EscrowError::MaxReleasesReached);

        // Calculate amounts
        let worker_amount = pool_escrow.payment_per_worker;
        let platform_fee = calculate_fee(worker_amount, pool_escrow.platform_fee_bps)?;
        let total_release = worker_amount.checked_add(platform_fee).ok_or(EscrowError::Overflow)?;

        // Verify vault has sufficient balance
        let remaining = pool_escrow.total_funded
            .checked_sub(pool_escrow.total_released)
            .ok_or(EscrowError::Overflow)?;
        require!(remaining >= total_release, EscrowError::InsufficientFunds);

        // Build signer seeds
        let escrow_id_bytes = pool_escrow.escrow_id.to_le_bytes();
        let seeds = &[
            POOL_ESCROW_SEED,
            pool_escrow.client.as_ref(),
            &escrow_id_bytes,
            &[pool_escrow.bump],
        ];
        let signer_seeds = &[&seeds[..]];

        // Transfer worker amount
        if worker_amount > 0 {
            let cpi_accounts = Transfer {
                from: ctx.accounts.vault.to_account_info(),
                to: ctx.accounts.worker_token_account.to_account_info(),
                authority: pool_escrow.to_account_info(),
            };
            let cpi_ctx = CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                cpi_accounts,
                signer_seeds,
            );
            token::transfer(cpi_ctx, worker_amount)?;
        }

        // Transfer platform fee to treasury
        if platform_fee > 0 {
            let cpi_accounts = Transfer {
                from: ctx.accounts.vault.to_account_info(),
                to: ctx.accounts.treasury_token_account.to_account_info(),
                authority: pool_escrow.to_account_info(),
            };
            let cpi_ctx = CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                cpi_accounts,
                signer_seeds,
            );
            token::transfer(cpi_ctx, platform_fee)?;
        }

        // Update tracking
        pool_escrow.total_released = pool_escrow.total_released
            .checked_add(total_release)
            .ok_or(EscrowError::Overflow)?;
        pool_escrow.release_count = pool_escrow.release_count
            .checked_add(1)
            .ok_or(EscrowError::Overflow)?;
        pool_escrow.status = PoolEscrowStatus::Active;

        let remaining_balance = pool_escrow.total_funded
            .checked_sub(pool_escrow.total_released)
            .ok_or(EscrowError::Overflow)?;

        emit!(PartialReleaseEvent {
            escrow: pool_escrow.key(),
            worker: ctx.accounts.worker_token_account.owner,
            worker_amount,
            platform_fee,
            release_number: pool_escrow.release_count,
            remaining_balance,
        });

        Ok(())
    }

    /// Close the pool escrow and refund remaining balance to client
    pub fn close_pool_escrow(ctx: Context<ClosePoolEscrow>) -> Result<()> {
        let pool_escrow = &mut ctx.accounts.pool_escrow;

        // Can only close funded or active escrows
        require!(
            pool_escrow.status == PoolEscrowStatus::Funded || 
            pool_escrow.status == PoolEscrowStatus::Active,
            EscrowError::InvalidStatus
        );
        require!(ctx.accounts.client.key() == pool_escrow.client, EscrowError::Unauthorized);

        // Calculate refund amount
        let remaining = pool_escrow.total_funded
            .checked_sub(pool_escrow.total_released)
            .ok_or(EscrowError::Overflow)?;

        if remaining > 0 {
            // Build signer seeds
            let escrow_id_bytes = pool_escrow.escrow_id.to_le_bytes();
            let seeds = &[
                POOL_ESCROW_SEED,
                pool_escrow.client.as_ref(),
                &escrow_id_bytes,
                &[pool_escrow.bump],
            ];
            let signer_seeds = &[&seeds[..]];

            // Transfer remaining to client
            let cpi_accounts = Transfer {
                from: ctx.accounts.vault.to_account_info(),
                to: ctx.accounts.client_token_account.to_account_info(),
                authority: pool_escrow.to_account_info(),
            };
            let cpi_ctx = CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                cpi_accounts,
                signer_seeds,
            );
            token::transfer(cpi_ctx, remaining)?;
        }

        pool_escrow.status = PoolEscrowStatus::Closed;
        pool_escrow.closed_at = Some(Clock::get()?.unix_timestamp);

        emit!(PoolEscrowClosedEvent {
            escrow: pool_escrow.key(),
            refund_amount: remaining,
            total_released: pool_escrow.total_released,
            release_count: pool_escrow.release_count,
        });

        Ok(())
    }
}


// ============================================================================
// HELPER FUNCTIONS
// ============================================================================

fn calculate_fee(amount: u64, fee_bps: u64) -> Result<u64> {
    let fee = (amount as u128)
        .checked_mul(fee_bps as u128)
        .ok_or(EscrowError::Overflow)?
        .checked_div(BPS_DENOMINATOR as u128)
        .ok_or(EscrowError::Overflow)?;
    
    if fee > u64::MAX as u128 {
        return Err(EscrowError::Overflow.into());
    }
    Ok(fee as u64)
}

// ============================================================================
// ACCOUNT STRUCTURES
// ============================================================================

#[account]
pub struct PlatformConfig {
    pub admin: Pubkey,
    pub treasury: Pubkey,
    /// Platform authority for partial_release (backend hot wallet)
    /// Separate from admin for security - if compromised, can only approve tasks, not change config
    pub platform_authority: Pubkey,
    pub paused: bool,
    /// Pending admin for two-step transfer
    pub pending_admin: Option<Pubkey>,
    pub bump: u8,
}

impl PlatformConfig {
    // Updated size - removed fee fields, added platform_authority
    // 8 (discriminator) + 32 (admin) + 32 (treasury) + 32 (platform_authority) + 1 (paused) + 33 (Option<Pubkey>) + 1 (bump) = 139
    pub const SIZE: usize = 8 + 32 + 32 + 32 + 1 + 33 + 1;
}

/// Type of escrow - determines which fee rate applies
#[derive(Clone, Copy, Debug, PartialEq, Eq, AnchorSerialize, AnchorDeserialize)]
pub enum EscrowType {
    /// Tasks, Gigs, Projects - 10% fee
    Task,
    /// Employment/Jobs - 5% fee  
    Employment,
}

impl Default for EscrowType {
    fn default() -> Self { EscrowType::Task }
}

#[account]
pub struct EscrowAccount {
    pub escrow_id: u64,
    pub client: Pubkey,
    pub worker: Pubkey,
    pub mint: Pubkey,
    pub vault: Pubkey,
    /// Amount the worker receives (the advertised payment)
    pub worker_amount: u64,
    /// Total amount in vault (worker_amount + platform_fee) - NEW for upfront fee model
    pub total_amount: u64,
    pub platform_fee_bps: u64,
    /// Type of escrow (Task=10% fee, Employment=5% fee)
    pub escrow_type: EscrowType,
    pub status: EscrowStatus,
    pub created_at: i64,
    pub funded_at: Option<i64>,
    pub released_at: Option<i64>,
    pub refunded_at: Option<i64>,
    pub frozen_at: Option<i64>,
    pub deadline: Option<i64>,
    pub bump: u8,
    pub vault_bump: u8,
    /// Version for backward compatibility (0 = legacy fee-at-release, 1 = upfront fee)
    pub version: u8,
}

impl EscrowAccount {
    // Updated SIZE: added 8 bytes for total_amount, 1 byte for version
    // 8 (discriminator) + 8 (escrow_id) + 32*4 (pubkeys) + 8 (worker_amount) + 8 (total_amount) + 
    // 8 (fee_bps) + 1 (escrow_type) + 1 (status) + 8 (created_at) + 9*5 (Option<i64>) + 
    // 1 (bump) + 1 (vault_bump) + 1 (version)
    pub const SIZE: usize = 8 + 8 + 32 + 32 + 32 + 32 + 8 + 8 + 8 + 1 + 1 + 8 + 9 + 9 + 9 + 9 + 9 + 1 + 1 + 1;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, AnchorSerialize, AnchorDeserialize)]
pub enum EscrowStatus {
    Created,
    Funded,
    Released,
    Refunded,
    Frozen,
}

impl Default for EscrowStatus {
    fn default() -> Self { EscrowStatus::Created }
}

impl fmt::Display for EscrowStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EscrowStatus::Created => write!(f, "Created"),
            EscrowStatus::Funded => write!(f, "Funded"),
            EscrowStatus::Released => write!(f, "Released"),
            EscrowStatus::Refunded => write!(f, "Refunded"),
            EscrowStatus::Frozen => write!(f, "Frozen"),
        }
    }
}

// ============================================================================
// POOL ESCROW ACCOUNT (Multi-Worker Tasks)
// ============================================================================

/// Pool escrow account for multi-worker tasks (microtask/crowdsourcing model)
/// Allows a client to fund a task that multiple workers can complete independently
#[account]
pub struct PoolEscrowAccount {
    /// Unique identifier for this pool escrow
    pub escrow_id: u64,
    /// Client who created and funded the pool escrow
    pub client: Pubkey,
    /// Token mint (USDC)
    pub mint: Pubkey,
    /// Vault token account holding the funds
    pub vault: Pubkey,
    /// Amount each worker receives upon approval
    pub payment_per_worker: u64,
    /// Maximum number of workers (releases) allowed
    pub max_releases: u64,
    /// Total amount funded (worker_budget + platform_fee)
    pub total_funded: u64,
    /// Total amount released so far (to workers + treasury)
    pub total_released: u64,
    /// Number of releases executed
    pub release_count: u64,
    /// Platform fee rate in basis points (1000 = 10%)
    pub platform_fee_bps: u64,
    /// Current status of the pool escrow
    pub status: PoolEscrowStatus,
    /// Timestamp when created
    pub created_at: i64,
    /// Timestamp when funded (None if not yet funded)
    pub funded_at: Option<i64>,
    /// Timestamp when closed (None if not yet closed)
    pub closed_at: Option<i64>,
    /// Optional deadline for task completion
    pub deadline: Option<i64>,
    /// PDA bump seed
    pub bump: u8,
    /// Vault PDA bump seed
    pub vault_bump: u8,
}

impl PoolEscrowAccount {
    // 8 (discriminator) + 8 (escrow_id) + 32*4 (pubkeys) + 8*6 (u64 fields) + 
    // 1 (status) + 8 (created_at) + 9*3 (Option<i64>) + 1 (bump) + 1 (vault_bump)
    pub const SIZE: usize = 8 + 8 + 32 + 32 + 32 + 32 + 8 + 8 + 8 + 8 + 8 + 8 + 1 + 8 + 9 + 9 + 9 + 1 + 1;
}

/// Status of a pool escrow
#[derive(Clone, Copy, Debug, PartialEq, Eq, AnchorSerialize, AnchorDeserialize)]
pub enum PoolEscrowStatus {
    /// Created but not yet funded
    Created,
    /// Funded and ready for releases
    Funded,
    /// At least one release has occurred
    Active,
    /// Closed and refunded
    Closed,
}

impl Default for PoolEscrowStatus {
    fn default() -> Self { PoolEscrowStatus::Created }
}

impl fmt::Display for PoolEscrowStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PoolEscrowStatus::Created => write!(f, "Created"),
            PoolEscrowStatus::Funded => write!(f, "Funded"),
            PoolEscrowStatus::Active => write!(f, "Active"),
            PoolEscrowStatus::Closed => write!(f, "Closed"),
        }
    }
}


// ============================================================================
// CONTEXT STRUCTURES
// ============================================================================

#[derive(Accounts)]
pub struct InitializeConfig<'info> {
    #[account(init, payer = admin, space = PlatformConfig::SIZE, seeds = [b"config"], bump)]
    pub config: Account<'info, PlatformConfig>,
    #[account(mut)]
    pub admin: Signer<'info>,
    /// Treasury wallet address (validated as a valid pubkey, token account validation happens at release time)
    /// CHECK: This is the treasury wallet pubkey. We validate it's not the zero address.
    #[account(constraint = treasury.key() != Pubkey::default() @ EscrowError::InvalidTreasury)]
    pub treasury: UncheckedAccount<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct UpdateConfig<'info> {
    #[account(mut, seeds = [b"config"], bump = config.bump, has_one = admin @ EscrowError::Unauthorized)]
    pub config: Account<'info, PlatformConfig>,
    pub admin: Signer<'info>,
}

/// Context for proposing a new admin
#[derive(Accounts)]
pub struct ProposeAdmin<'info> {
    #[account(mut, seeds = [b"config"], bump = config.bump, has_one = admin @ EscrowError::Unauthorized)]
    pub config: Account<'info, PlatformConfig>,
    pub admin: Signer<'info>,
}

/// Context for accepting admin role
#[derive(Accounts)]
pub struct AcceptAdmin<'info> {
    #[account(mut, seeds = [b"config"], bump = config.bump)]
    pub config: Account<'info, PlatformConfig>,
    pub new_admin: Signer<'info>,
}

#[derive(Accounts)]
#[instruction(escrow_id: u64)]
pub struct CreateEscrow<'info> {
    #[account(seeds = [b"config"], bump = config.bump)]
    pub config: Account<'info, PlatformConfig>,
    #[account(init, payer = client, space = EscrowAccount::SIZE, seeds = [ESCROW_SEED, client.key().as_ref(), worker.key().as_ref(), &escrow_id.to_le_bytes()], bump)]
    pub escrow: Account<'info, EscrowAccount>,
    #[account(init, payer = client, token::mint = mint, token::authority = escrow, seeds = [VAULT_SEED, escrow.key().as_ref()], bump)]
    pub vault: Account<'info, TokenAccount>,
    pub mint: Account<'info, Mint>,
    #[account(mut)]
    pub client: Signer<'info>,
    /// CHECK: Worker account - validated by constraint
    #[account(constraint = worker.key() != client.key() @ EscrowError::SameClientWorker)]
    pub worker: UncheckedAccount<'info>,
    pub system_program: Program<'info, System>,
    #[account(address = token::ID)]
    pub token_program: Program<'info, Token>,
    pub rent: Sysvar<'info, Rent>,
}

#[derive(Accounts)]
pub struct FundEscrow<'info> {
    #[account(mut, seeds = [ESCROW_SEED, escrow.client.as_ref(), escrow.worker.as_ref(), &escrow.escrow_id.to_le_bytes()], bump = escrow.bump, has_one = client @ EscrowError::Unauthorized, has_one = vault @ EscrowError::InvalidVault, has_one = mint @ EscrowError::InvalidMint)]
    pub escrow: Account<'info, EscrowAccount>,
    #[account(mut, seeds = [VAULT_SEED, escrow.key().as_ref()], bump = escrow.vault_bump)]
    pub vault: Account<'info, TokenAccount>,
    #[account(mut, token::mint = mint, token::authority = client)]
    pub client_token_account: Account<'info, TokenAccount>,
    pub mint: Account<'info, Mint>,
    pub client: Signer<'info>,
    #[account(address = token::ID)]
    pub token_program: Program<'info, Token>,
}


#[derive(Accounts)]
pub struct ReleaseEscrow<'info> {
    #[account(seeds = [b"config"], bump = config.bump)]
    pub config: Account<'info, PlatformConfig>,
    #[account(mut, seeds = [ESCROW_SEED, escrow.client.as_ref(), escrow.worker.as_ref(), &escrow.escrow_id.to_le_bytes()], bump = escrow.bump, has_one = client @ EscrowError::Unauthorized, has_one = vault @ EscrowError::InvalidVault)]
    pub escrow: Account<'info, EscrowAccount>,
    #[account(mut, seeds = [VAULT_SEED, escrow.key().as_ref()], bump = escrow.vault_bump)]
    pub vault: Account<'info, TokenAccount>,
    /// Prevent duplicate mutable accounts
    #[account(mut, token::mint = escrow.mint, token::authority = escrow.worker, constraint = worker_token_account.key() != treasury_token_account.key() @ EscrowError::DuplicateAccounts)]
    pub worker_token_account: Account<'info, TokenAccount>,
    #[account(mut, token::mint = escrow.mint, constraint = treasury_token_account.owner == config.treasury @ EscrowError::InvalidTreasury)]
    pub treasury_token_account: Account<'info, TokenAccount>,
    pub client: Signer<'info>,
    #[account(address = token::ID)]
    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct FreezeEscrow<'info> {
    #[account(seeds = [b"config"], bump = config.bump)]
    pub config: Account<'info, PlatformConfig>,
    #[account(mut, seeds = [ESCROW_SEED, escrow.client.as_ref(), escrow.worker.as_ref(), &escrow.escrow_id.to_le_bytes()], bump = escrow.bump)]
    pub escrow: Account<'info, EscrowAccount>,
    pub caller: Signer<'info>,
}

#[derive(Accounts)]
pub struct AdminActionCtx<'info> {
    #[account(seeds = [b"config"], bump = config.bump, has_one = admin @ EscrowError::Unauthorized)]
    pub config: Account<'info, PlatformConfig>,
    #[account(mut, seeds = [ESCROW_SEED, escrow.client.as_ref(), escrow.worker.as_ref(), &escrow.escrow_id.to_le_bytes()], bump = escrow.bump, has_one = vault @ EscrowError::InvalidVault)]
    pub escrow: Account<'info, EscrowAccount>,
    #[account(mut, seeds = [VAULT_SEED, escrow.key().as_ref()], bump = escrow.vault_bump)]
    pub vault: Account<'info, TokenAccount>,
    /// Prevent duplicate mutable accounts
    #[account(mut, token::mint = escrow.mint, token::authority = escrow.worker, constraint = worker_token_account.key() != client_token_account.key() @ EscrowError::DuplicateAccounts)]
    pub worker_token_account: Account<'info, TokenAccount>,
    #[account(mut, token::mint = escrow.mint, token::authority = escrow.client)]
    pub client_token_account: Account<'info, TokenAccount>,
    /// Treasury token account for fee collection during disputes
    #[account(
        mut,
        token::mint = escrow.mint,
        constraint = treasury_token_account.owner == config.treasury @ EscrowError::InvalidTreasury,
        constraint = treasury_token_account.key() != worker_token_account.key() @ EscrowError::DuplicateAccounts,
        constraint = treasury_token_account.key() != client_token_account.key() @ EscrowError::DuplicateAccounts
    )]
    pub treasury_token_account: Account<'info, TokenAccount>,
    pub admin: Signer<'info>,
    #[account(address = token::ID)]
    pub token_program: Program<'info, Token>,
}

/// Context for admin refund to client (dispute resolution - client wins)
/// Simpler than AdminActionCtx - no treasury needed since client gets FULL refund
#[derive(Accounts)]
pub struct AdminRefundCtx<'info> {
    #[account(seeds = [b"config"], bump = config.bump, has_one = admin @ EscrowError::Unauthorized)]
    pub config: Account<'info, PlatformConfig>,
    #[account(mut, seeds = [ESCROW_SEED, escrow.client.as_ref(), escrow.worker.as_ref(), &escrow.escrow_id.to_le_bytes()], bump = escrow.bump, has_one = vault @ EscrowError::InvalidVault)]
    pub escrow: Account<'info, EscrowAccount>,
    #[account(mut, seeds = [VAULT_SEED, escrow.key().as_ref()], bump = escrow.vault_bump)]
    pub vault: Account<'info, TokenAccount>,
    #[account(mut, token::mint = escrow.mint, token::authority = escrow.client)]
    pub client_token_account: Account<'info, TokenAccount>,
    pub admin: Signer<'info>,
    #[account(address = token::ID)]
    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct RefundEscrow<'info> {
    #[account(mut, seeds = [ESCROW_SEED, escrow.client.as_ref(), escrow.worker.as_ref(), &escrow.escrow_id.to_le_bytes()], bump = escrow.bump, has_one = client @ EscrowError::Unauthorized, has_one = vault @ EscrowError::InvalidVault)]
    pub escrow: Account<'info, EscrowAccount>,
    #[account(mut, seeds = [VAULT_SEED, escrow.key().as_ref()], bump = escrow.vault_bump)]
    pub vault: Account<'info, TokenAccount>,
    #[account(mut, token::mint = escrow.mint, token::authority = client)]
    pub client_token_account: Account<'info, TokenAccount>,
    pub client: Signer<'info>,
    #[account(address = token::ID)]
    pub token_program: Program<'info, Token>,
}


#[derive(Accounts)]
pub struct CancelEscrow<'info> {
    #[account(mut, seeds = [ESCROW_SEED, escrow.client.as_ref(), escrow.worker.as_ref(), &escrow.escrow_id.to_le_bytes()], bump = escrow.bump, has_one = client @ EscrowError::Unauthorized, close = client)]
    pub escrow: Account<'info, EscrowAccount>,
    /// Vault is closed via CPI to Token Program (not Anchor's close constraint)
    #[account(mut, seeds = [VAULT_SEED, escrow.key().as_ref()], bump = escrow.vault_bump)]
    pub vault: Account<'info, TokenAccount>,
    #[account(mut)]
    pub client: Signer<'info>,
    #[account(address = token::ID)]
    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct AdminSplit<'info> {
    #[account(seeds = [b"config"], bump = config.bump, has_one = admin @ EscrowError::Unauthorized)]
    pub config: Account<'info, PlatformConfig>,
    #[account(mut, seeds = [ESCROW_SEED, escrow.client.as_ref(), escrow.worker.as_ref(), &escrow.escrow_id.to_le_bytes()], bump = escrow.bump, has_one = vault @ EscrowError::InvalidVault)]
    pub escrow: Account<'info, EscrowAccount>,
    #[account(mut, seeds = [VAULT_SEED, escrow.key().as_ref()], bump = escrow.vault_bump)]
    pub vault: Account<'info, TokenAccount>,
    /// Prevent duplicate mutable accounts
    #[account(mut, token::mint = escrow.mint, token::authority = escrow.worker, constraint = worker_token_account.key() != client_token_account.key() @ EscrowError::DuplicateAccounts)]
    pub worker_token_account: Account<'info, TokenAccount>,
    #[account(mut, token::mint = escrow.mint, token::authority = escrow.client)]
    pub client_token_account: Account<'info, TokenAccount>,
    /// Treasury token account for fee collection during disputes
    #[account(
        mut,
        token::mint = escrow.mint,
        constraint = treasury_token_account.owner == config.treasury @ EscrowError::InvalidTreasury,
        constraint = treasury_token_account.key() != worker_token_account.key() @ EscrowError::DuplicateAccounts,
        constraint = treasury_token_account.key() != client_token_account.key() @ EscrowError::DuplicateAccounts
    )]
    pub treasury_token_account: Account<'info, TokenAccount>,
    pub admin: Signer<'info>,
    #[account(address = token::ID)]
    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct CloseEscrow<'info> {
    #[account(mut, seeds = [ESCROW_SEED, escrow.client.as_ref(), escrow.worker.as_ref(), &escrow.escrow_id.to_le_bytes()], bump = escrow.bump, has_one = client @ EscrowError::Unauthorized, close = client)]
    pub escrow: Account<'info, EscrowAccount>,
    /// Vault is closed via CPI to Token Program (not Anchor's close constraint)
    #[account(mut, seeds = [VAULT_SEED, escrow.key().as_ref()], bump = escrow.vault_bump)]
    pub vault: Account<'info, TokenAccount>,
    #[account(mut)]
    pub client: Signer<'info>,
    #[account(address = token::ID)]
    pub token_program: Program<'info, Token>,
}

// ============================================================================
// POOL ESCROW CONTEXT STRUCTURES (Multi-Worker Tasks)
// ============================================================================

#[derive(Accounts)]
#[instruction(escrow_id: u64)]
pub struct CreatePoolEscrow<'info> {
    #[account(seeds = [b"config"], bump = config.bump)]
    pub config: Account<'info, PlatformConfig>,
    #[account(
        init,
        payer = client,
        space = PoolEscrowAccount::SIZE,
        seeds = [POOL_ESCROW_SEED, client.key().as_ref(), &escrow_id.to_le_bytes()],
        bump
    )]
    pub pool_escrow: Account<'info, PoolEscrowAccount>,
    #[account(
        init,
        payer = client,
        token::mint = mint,
        token::authority = pool_escrow,
        seeds = [POOL_VAULT_SEED, pool_escrow.key().as_ref()],
        bump
    )]
    pub vault: Account<'info, TokenAccount>,
    pub mint: Account<'info, Mint>,
    #[account(mut)]
    pub client: Signer<'info>,
    pub system_program: Program<'info, System>,
    #[account(address = token::ID)]
    pub token_program: Program<'info, Token>,
    pub rent: Sysvar<'info, Rent>,
}

#[derive(Accounts)]
pub struct FundPoolEscrow<'info> {
    #[account(
        mut,
        seeds = [POOL_ESCROW_SEED, pool_escrow.client.as_ref(), &pool_escrow.escrow_id.to_le_bytes()],
        bump = pool_escrow.bump,
        has_one = client @ EscrowError::Unauthorized,
        has_one = vault @ EscrowError::InvalidVault,
        has_one = mint @ EscrowError::InvalidMint
    )]
    pub pool_escrow: Account<'info, PoolEscrowAccount>,
    #[account(mut, seeds = [POOL_VAULT_SEED, pool_escrow.key().as_ref()], bump = pool_escrow.vault_bump)]
    pub vault: Account<'info, TokenAccount>,
    #[account(mut, token::mint = mint, token::authority = client)]
    pub client_token_account: Account<'info, TokenAccount>,
    pub mint: Account<'info, Mint>,
    pub client: Signer<'info>,
    #[account(address = token::ID)]
    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct PartialRelease<'info> {
    #[account(
        seeds = [b"config"],
        bump = config.bump,
        has_one = platform_authority @ EscrowError::Unauthorized  // Verify signer is platform authority
    )]
    pub config: Account<'info, PlatformConfig>,
    #[account(
        mut,
        seeds = [POOL_ESCROW_SEED, pool_escrow.client.as_ref(), &pool_escrow.escrow_id.to_le_bytes()],
        bump = pool_escrow.bump,
        has_one = vault @ EscrowError::InvalidVault
    )]
    pub pool_escrow: Account<'info, PoolEscrowAccount>,
    #[account(mut, seeds = [POOL_VAULT_SEED, pool_escrow.key().as_ref()], bump = pool_escrow.vault_bump)]
    pub vault: Account<'info, TokenAccount>,
    /// Worker's token account to receive payment
    #[account(
        mut,
        token::mint = pool_escrow.mint,
        constraint = worker_token_account.key() != treasury_token_account.key() @ EscrowError::DuplicateAccounts
    )]
    pub worker_token_account: Account<'info, TokenAccount>,
    /// Treasury token account to receive platform fee
    #[account(
        mut,
        token::mint = pool_escrow.mint,
        constraint = treasury_token_account.owner == config.treasury @ EscrowError::InvalidTreasury
    )]
    pub treasury_token_account: Account<'info, TokenAccount>,
    /// Platform authority signer (NOT admin - separate hot wallet for backend)
    /// Only this key can call partial_release, preventing unauthorized fund drainage
    pub platform_authority: Signer<'info>,
    #[account(address = token::ID)]
    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct ClosePoolEscrow<'info> {
    #[account(
        mut,
        seeds = [POOL_ESCROW_SEED, pool_escrow.client.as_ref(), &pool_escrow.escrow_id.to_le_bytes()],
        bump = pool_escrow.bump,
        has_one = client @ EscrowError::Unauthorized,
        has_one = vault @ EscrowError::InvalidVault,
        close = client  // Return rent to client when closing pool escrow
    )]
    pub pool_escrow: Account<'info, PoolEscrowAccount>,
    #[account(mut, seeds = [POOL_VAULT_SEED, pool_escrow.key().as_ref()], bump = pool_escrow.vault_bump)]
    pub vault: Account<'info, TokenAccount>,
    #[account(mut, token::mint = pool_escrow.mint, token::authority = client)]
    pub client_token_account: Account<'info, TokenAccount>,
    #[account(mut)]
    pub client: Signer<'info>,
    #[account(address = token::ID)]
    pub token_program: Program<'info, Token>,
}


// ============================================================================
// EVENTS
// ============================================================================

#[event]
pub struct ConfigInitialized {
    pub admin: Pubkey,
    pub treasury: Pubkey,
    pub platform_authority: Pubkey,
}

#[event]
pub struct ConfigUpdated {
    pub admin: Pubkey,
    pub treasury: Pubkey,
    pub platform_authority: Pubkey,
    pub paused: bool,
}

#[event]
pub struct AdminProposed {
    pub current_admin: Pubkey,
    pub proposed_admin: Pubkey,
    pub proposed_at: i64,
}

#[event]
pub struct AdminTransferred {
    pub old_admin: Pubkey,
    pub new_admin: Pubkey,
    pub transferred_at: i64,
}

#[event]
pub struct AdminTransferCancelled {
    pub admin: Pubkey,
    pub cancelled_pending: Pubkey,
    pub cancelled_at: i64,
}

#[event]
pub struct EscrowCreated {
    pub escrow: Pubkey,
    pub escrow_id: u64,
    pub client: Pubkey,
    pub worker: Pubkey,
    pub mint: Pubkey,
    pub amount: u64,
    pub escrow_type: EscrowType,
    pub deadline: Option<i64>,
}

#[event]
pub struct EscrowFunded {
    pub escrow: Pubkey,
    pub amount: u64,
    pub funded_at: i64,
}

#[event]
pub struct EscrowReleased {
    pub escrow: Pubkey,
    pub worker_amount: u64,
    pub platform_fee: u64,
    pub released_at: i64,
}

#[event]
pub struct EscrowFrozen {
    pub escrow: Pubkey,
    pub frozen_by: Pubkey,
    pub frozen_at: i64,
}

#[event]
pub struct AdminActionEvent {
    pub escrow: Pubkey,
    pub action: String,
    pub amount: u64,
    pub admin: Pubkey,
    pub timestamp: i64,
}

#[event]
pub struct EscrowRefunded {
    pub escrow: Pubkey,
    pub client: Pubkey,
    pub amount: u64,
    pub refunded_at: i64,
    pub reason: String,
}

#[event]
pub struct EscrowCancelled {
    pub escrow: Pubkey,
    pub client: Pubkey,
    pub cancelled_at: i64,
}

#[event]
pub struct AdminSplitFundsEvent {
    pub escrow: Pubkey,
    pub worker_amount: u64,
    pub client_amount: u64,
    pub worker_percentage: u64,
    pub admin: Pubkey,
    pub timestamp: i64,
}

#[event]
pub struct EscrowClosed {
    pub escrow: Pubkey,
    pub closed_at: i64,
}

// ============================================================================
// POOL ESCROW EVENTS (Multi-Worker Tasks)
// ============================================================================

#[event]
pub struct PoolEscrowCreated {
    pub escrow: Pubkey,
    pub escrow_id: u64,
    pub client: Pubkey,
    pub mint: Pubkey,
    pub payment_per_worker: u64,
    pub max_releases: u64,
    pub total_funded: u64,
    pub deadline: Option<i64>,
}

#[event]
pub struct PoolEscrowFunded {
    pub escrow: Pubkey,
    pub amount: u64,
    pub funded_at: i64,
}

#[event]
pub struct PartialReleaseEvent {
    pub escrow: Pubkey,
    pub worker: Pubkey,
    pub worker_amount: u64,
    pub platform_fee: u64,
    pub release_number: u64,
    pub remaining_balance: u64,
}

#[event]
pub struct PoolEscrowClosedEvent {
    pub escrow: Pubkey,
    pub refund_amount: u64,
    pub total_released: u64,
    pub release_count: u64,
}


// ============================================================================
// ERRORS
// ============================================================================

#[error_code]
pub enum EscrowError {
    #[msg("Invalid escrow status for this operation")]
    InvalidStatus,
    #[msg("Unauthorized: only authorized parties can perform this action")]
    Unauthorized,
    #[msg("Insufficient funds in escrow")]
    InsufficientFunds,
    #[msg("Deadline has not passed yet")]
    DeadlineNotPassed,
    #[msg("No deadline set for this escrow")]
    NoDeadlineSet,
    #[msg("Invalid percentage value (must be 0-10000 basis points)")]
    InvalidPercentage,
    #[msg("Amount is below minimum escrow amount")]
    AmountTooSmall,
    #[msg("Arithmetic overflow in calculation")]
    Overflow,
    #[msg("Platform is currently paused")]
    PlatformPaused,
    #[msg("Invalid vault account")]
    InvalidVault,
    #[msg("Invalid mint account")]
    InvalidMint,
    #[msg("Invalid treasury account")]
    InvalidTreasury,
    #[msg("Client and worker cannot be the same")]
    SameClientWorker,
    #[msg("Vault is not empty - cannot close")]
    VaultNotEmpty,
    #[msg("Invalid admin address")]
    InvalidAdmin,
    #[msg("No pending admin transfer")]
    NoPendingAdmin,
    #[msg("Deadline must be in the future")]
    DeadlineInPast,
    #[msg("Deadline is too far in the future (max 1 year)")]
    DeadlineTooFar,
    #[msg("Escrow was never funded")]
    NotFunded,
    #[msg("Duplicate mutable accounts not allowed")]
    DuplicateAccounts,
    #[msg("Invalid max_releases value (must be 1-10000)")]
    InvalidMaxReleases,
    #[msg("Maximum number of releases reached")]
    MaxReleasesReached,
    #[msg("Invalid worker address: pubkey is not on the ed25519 curve")]
    InvalidWorkerAddress,
    #[msg("Invalid platform authority address")]
    InvalidPlatformAuthority,
    #[msg("Deadline has passed - no more releases allowed")]
    DeadlinePassed,
}
