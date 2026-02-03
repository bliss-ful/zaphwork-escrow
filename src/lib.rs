use anchor_lang::prelude::*;
use anchor_spl::token::{self, CloseAccount, Mint, Token, TokenAccount, Transfer};
use std::collections::BTreeSet;
use std::fmt;

declare_id!("3iKABSF5zoQjGPykxUQNxbm7eQADqs8DreuGupggc679");

// ============================================================================
// CONSTANTS - CORE PROTOCOL
// ============================================================================

/// Minimum escrow amount (1 USDC = 1_000_000 with 6 decimals)
pub const MIN_ESCROW_AMOUNT: u64 = 1_000_000;

/// Basis points denominator (changed to u16 for consistency)
pub const BPS_DENOMINATOR: u16 = 10_000;

/// Escrow seed prefix
pub const ESCROW_SEED: &[u8] = b"escrow";

/// Vault seed prefix
pub const VAULT_SEED: &[u8] = b"vault";

/// Pool escrow seed prefix (for multi-worker tasks)
pub const POOL_ESCROW_SEED: &[u8] = b"pool_escrow";

/// Pool vault seed prefix
pub const POOL_VAULT_SEED: &[u8] = b"pool_vault";

/// Maximum number of workers for a pool escrow
pub const MAX_POOL_WORKERS: u64 = 10_000;

/// Maximum escrow duration (1 year in seconds)
pub const MAX_ESCROW_DURATION: i64 = 365 * 24 * 60 * 60;

/// Maximum number of split recipients
pub const MAX_SPLITS: usize = 8;

// ============================================================================
// PROGRAM MODULE
// ============================================================================

#[program]
pub mod zaphwork {
    use super::*;

    // ========================================================================
    // CONFIGURATION MANAGEMENT
    // ========================================================================

    /// Initialize the platform config (one-time setup)
    pub fn initialize_config(ctx: Context<InitializeConfig>, treasury: Pubkey) -> Result<()> {
        require!(treasury != Pubkey::default(), EscrowError::InvalidTreasury);
        let config = &mut ctx.accounts.config;
        config.admin = ctx.accounts.admin.key();
        config.treasury = treasury;
        config.paused = false;
        config.pending_admin = None;
        config.bump = ctx.bumps.config;
        Ok(())
    }

    /// Update platform config (admin only)
    pub fn update_config(
        ctx: Context<UpdateConfig>,
        new_treasury: Option<Pubkey>,
        paused: Option<bool>,
    ) -> Result<()> {
        let config = &mut ctx.accounts.config;
        if let Some(treasury) = new_treasury {
            require!(treasury != Pubkey::default(), EscrowError::InvalidTreasury);
            config.treasury = treasury;
        }
        if let Some(is_paused) = paused {
            config.paused = is_paused;
        }
        Ok(())
    }

    /// Propose a new admin (two-step transfer for safety)
    pub fn propose_admin(ctx: Context<ProposeAdmin>, new_admin: Pubkey) -> Result<()> {
        require!(new_admin != Pubkey::default(), EscrowError::InvalidAdmin);
        let config = &mut ctx.accounts.config;
        config.pending_admin = Some(new_admin);
        Ok(())
    }

    /// Accept admin role (must be called by the pending admin)
    pub fn accept_admin(ctx: Context<AcceptAdmin>) -> Result<()> {
        let config = &mut ctx.accounts.config;
        let pending = config.pending_admin.ok_or(EscrowError::NoPendingAdmin)?;
        require!(ctx.accounts.new_admin.key() == pending, EscrowError::Unauthorized);
        config.admin = pending;
        config.pending_admin = None;
        Ok(())
    }

    /// Cancel a pending admin transfer (current admin only)
    pub fn cancel_admin_transfer(ctx: Context<UpdateConfig>) -> Result<()> {
        let config = &mut ctx.accounts.config;
        require!(config.pending_admin.is_some(), EscrowError::NoPendingAdmin);
        config.pending_admin = None;
        Ok(())
    }

    // ========================================================================
    // CORE ESCROW INSTRUCTIONS
    // ========================================================================

    /// Create escrow with split-based settlement
    /// Caller provides splits that define how funds will be distributed
    pub fn create_escrow(
        ctx: Context<CreateEscrow>,
        escrow_id: u64,
        total_amount: u64,
        splits: Vec<Split>,
        deadline: Option<i64>,
    ) -> Result<()> {
        let config = &ctx.accounts.config;
        require!(!config.paused, EscrowError::PlatformPaused);
        require!(total_amount >= MIN_ESCROW_AMOUNT, EscrowError::AmountTooSmall);

        if let Some(dl) = deadline {
            let now = Clock::get()?.unix_timestamp;
            require!(dl > now, EscrowError::DeadlineInPast);
            let max_deadline = now.checked_add(MAX_ESCROW_DURATION).ok_or(EscrowError::Overflow)?;
            require!(dl <= max_deadline, EscrowError::DeadlineTooFar);
        }

        validate_splits(&splits)?;

        let escrow = &mut ctx.accounts.escrow;
        escrow.escrow_id = escrow_id;
        escrow.payer = ctx.accounts.payer.key();
        escrow.mint = ctx.accounts.mint.key();
        escrow.vault = ctx.accounts.vault.key();
        escrow.total_amount = total_amount;
        escrow.splits = splits;
        escrow.status = EscrowStatus::Created;
        escrow.created_at = Clock::get()?.unix_timestamp;
        escrow.funded_at = None;
        escrow.approved_at = None;
        escrow.settled_at = None;
        escrow.refunded_at = None;
        escrow.frozen_at = None;
        escrow.deadline = deadline;
        escrow.bump = ctx.bumps.escrow;
        escrow.vault_bump = ctx.bumps.vault;
        escrow.version = 2;
        Ok(())
    }

    /// Fund the escrow with tokens
    pub fn fund_escrow(ctx: Context<FundEscrow>) -> Result<()> {
        let escrow = &mut ctx.accounts.escrow;
        require!(escrow.status == EscrowStatus::Created, EscrowError::InvalidStatus);
        require!(ctx.accounts.payer.key() == escrow.payer, EscrowError::Unauthorized);

        let cpi_accounts = Transfer {
            from: ctx.accounts.payer_token_account.to_account_info(),
            to: ctx.accounts.vault.to_account_info(),
            authority: ctx.accounts.payer.to_account_info(),
        };
        let cpi_ctx = CpiContext::new(ctx.accounts.token_program.to_account_info(), cpi_accounts);
        token::transfer(cpi_ctx, escrow.total_amount)?;

        escrow.status = EscrowStatus::Funded;
        escrow.funded_at = Some(Clock::get()?.unix_timestamp);
        Ok(())
    }

    /// Approve escrow (optional step before settlement)
    pub fn approve_escrow(ctx: Context<ApproveEscrow>) -> Result<()> {
        let escrow = &mut ctx.accounts.escrow;
        require!(escrow.status == EscrowStatus::Funded, EscrowError::InvalidStatus);
        require!(ctx.accounts.payer.key() == escrow.payer, EscrowError::Unauthorized);
        escrow.status = EscrowStatus::Approved;
        escrow.approved_at = Some(Clock::get()?.unix_timestamp);
        Ok(())
    }

    /// Settle escrow with split-based distribution
    /// Remaining accounts must be token accounts for each split recipient
    pub fn settle_escrow<'info>(
        ctx: Context<'_, '_, '_, 'info, SettleEscrow<'info>>,
    ) -> Result<()> {
        let status = ctx.accounts.escrow.status;
        require!(
            status == EscrowStatus::Approved || status == EscrowStatus::Funded,
            EscrowError::InvalidStatus
        );
        require!(ctx.accounts.payer.key() == ctx.accounts.escrow.payer, EscrowError::Unauthorized);

        let total_amount = ctx.accounts.escrow.total_amount;
        let mint = ctx.accounts.escrow.mint;
        let vault_key = ctx.accounts.vault.key();
        let splits = ctx.accounts.escrow.splits.clone();
        let split_amounts = compute_split_amounts(total_amount, &splits)?;

        require!(
            ctx.remaining_accounts.len() == splits.len(),
            EscrowError::InvalidRemainingAccounts
        );

        let mut seen = BTreeSet::<Pubkey>::new();
        for (i, split) in splits.iter().enumerate() {
            let ta_info = &ctx.remaining_accounts[i];
            require!(ta_info.is_writable, EscrowError::Unauthorized);
            require!(seen.insert(ta_info.key()), EscrowError::DuplicateAccounts);
            require!(ta_info.key() != vault_key, EscrowError::DuplicateAccounts);
            require!(*ta_info.owner == token::ID, EscrowError::InvalidVault);

            let mut data: &[u8] = &ta_info.try_borrow_data()?;
            let ta = TokenAccount::try_deserialize(&mut data)?;
            require!(ta.mint == mint, EscrowError::InvalidMint);
            require!(ta.owner == split.recipient, EscrowError::InvalidRecipientTokenAccount);
        }

        let escrow_id_bytes = ctx.accounts.escrow.escrow_id.to_le_bytes();
        let seeds = &[
            ESCROW_SEED,
            ctx.accounts.escrow.payer.as_ref(),
            &escrow_id_bytes,
            &[ctx.accounts.escrow.bump],
        ];
        let signer_seeds = &[&seeds[..]];
        let vault_info = ctx.accounts.vault.to_account_info();
        let escrow_info = ctx.accounts.escrow.to_account_info();
        let token_program_info = ctx.accounts.token_program.to_account_info();

        for i in 0..splits.len() {
            let amount = split_amounts[i];
            if amount == 0 {
                continue;
            }
            let to = ctx.remaining_accounts[i].clone();
            let cpi_accounts = Transfer {
                from: vault_info.clone(),
                to,
                authority: escrow_info.clone(),
            };
            let cpi_ctx = CpiContext::new_with_signer(
                token_program_info.clone(),
                cpi_accounts,
                signer_seeds,
            );
            token::transfer(cpi_ctx, amount)?;
        }

        let escrow = &mut ctx.accounts.escrow;
        escrow.status = EscrowStatus::Settled;
        escrow.settled_at = Some(Clock::get()?.unix_timestamp);
        Ok(())
    }

    /// Refund escrow to payer (deadline passed)
    pub fn refund_escrow(ctx: Context<RefundEscrow>) -> Result<()> {
        let escrow = &mut ctx.accounts.escrow;
        require!(
            escrow.status == EscrowStatus::Funded || escrow.status == EscrowStatus::Approved,
            EscrowError::InvalidStatus
        );
        require!(ctx.accounts.payer.key() == escrow.payer, EscrowError::Unauthorized);
        let deadline = escrow.deadline.ok_or(EscrowError::NoDeadlineSet)?;
        require!(Clock::get()?.unix_timestamp > deadline, EscrowError::DeadlineNotPassed);

        let escrow_id_bytes = escrow.escrow_id.to_le_bytes();
        let seeds = &[
            ESCROW_SEED,
            escrow.payer.as_ref(),
            &escrow_id_bytes,
            &[escrow.bump],
        ];
        let signer_seeds = &[&seeds[..]];

        let cpi_accounts = Transfer {
            from: ctx.accounts.vault.to_account_info(),
            to: ctx.accounts.payer_token_account.to_account_info(),
            authority: escrow.to_account_info(),
        };
        let cpi_ctx = CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            cpi_accounts,
            signer_seeds,
        );
        token::transfer(cpi_ctx, escrow.total_amount)?;

        escrow.status = EscrowStatus::Refunded;
        escrow.refunded_at = Some(Clock::get()?.unix_timestamp);
        Ok(())
    }

    // ========================================================================
    // DISPUTE LAYER - Admin functions for dispute resolution
    // ========================================================================

    /// Freeze escrow on dispute (payer, recipient, or admin can call)
    pub fn freeze_escrow(ctx: Context<FreezeEscrow>) -> Result<()> {
        let escrow = &mut ctx.accounts.escrow;
        require!(
            escrow.status == EscrowStatus::Funded || escrow.status == EscrowStatus::Approved,
            EscrowError::InvalidStatus
        );
        let caller = ctx.accounts.caller.key();
        let is_recipient = escrow.splits.iter().any(|s| s.recipient == caller);
        require!(
            caller == escrow.payer || caller == ctx.accounts.config.admin || is_recipient,
            EscrowError::Unauthorized
        );
        escrow.status = EscrowStatus::Frozen;
        escrow.frozen_at = Some(Clock::get()?.unix_timestamp);
        Ok(())
    }

    /// Admin refund to payer (dispute resolution)
    pub fn admin_refund_to_payer(ctx: Context<AdminRefundToPayer>) -> Result<()> {
        let escrow = &mut ctx.accounts.escrow;
        require!(escrow.status == EscrowStatus::Frozen, EscrowError::InvalidStatus);
        require!(ctx.accounts.admin.key() == ctx.accounts.config.admin, EscrowError::Unauthorized);

        let escrow_id_bytes = escrow.escrow_id.to_le_bytes();
        let seeds = &[
            ESCROW_SEED,
            escrow.payer.as_ref(),
            &escrow_id_bytes,
            &[escrow.bump],
        ];
        let signer_seeds = &[&seeds[..]];

        let cpi_accounts = Transfer {
            from: ctx.accounts.vault.to_account_info(),
            to: ctx.accounts.payer_token_account.to_account_info(),
            authority: escrow.to_account_info(),
        };
        let cpi_ctx = CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            cpi_accounts,
            signer_seeds,
        );
        token::transfer(cpi_ctx, escrow.total_amount)?;

        escrow.status = EscrowStatus::Refunded;
        escrow.refunded_at = Some(Clock::get()?.unix_timestamp);
        Ok(())
    }

    /// Admin settle with custom splits (dispute resolution)
    pub fn admin_settle_with_splits<'info>(
        ctx: Context<'_, '_, '_, 'info, AdminSettleEscrow<'info>>,
        splits: Vec<Split>,
    ) -> Result<()> {
        let status = ctx.accounts.escrow.status;
        require!(status == EscrowStatus::Frozen, EscrowError::InvalidStatus);
        require!(ctx.accounts.admin.key() == ctx.accounts.config.admin, EscrowError::Unauthorized);

        validate_splits(&splits)?;
        let total_amount = ctx.accounts.escrow.total_amount;
        let mint = ctx.accounts.escrow.mint;
        let vault_key = ctx.accounts.vault.key();
        let split_amounts = compute_split_amounts(total_amount, &splits)?;

        require!(
            ctx.remaining_accounts.len() == splits.len(),
            EscrowError::InvalidRemainingAccounts
        );

        let mut seen = BTreeSet::<Pubkey>::new();
        for (i, split) in splits.iter().enumerate() {
            let ta_info = &ctx.remaining_accounts[i];
            require!(ta_info.is_writable, EscrowError::Unauthorized);
            require!(seen.insert(ta_info.key()), EscrowError::DuplicateAccounts);
            require!(ta_info.key() != vault_key, EscrowError::DuplicateAccounts);
            require!(*ta_info.owner == token::ID, EscrowError::InvalidVault);

            let mut data: &[u8] = &ta_info.try_borrow_data()?;
            let ta = TokenAccount::try_deserialize(&mut data)?;
            require!(ta.mint == mint, EscrowError::InvalidMint);
            require!(ta.owner == split.recipient, EscrowError::InvalidRecipientTokenAccount);
        }

        let escrow_id_bytes = ctx.accounts.escrow.escrow_id.to_le_bytes();
        let seeds = &[
            ESCROW_SEED,
            ctx.accounts.escrow.payer.as_ref(),
            &escrow_id_bytes,
            &[ctx.accounts.escrow.bump],
        ];
        let signer_seeds = &[&seeds[..]];
        let vault_info = ctx.accounts.vault.to_account_info();
        let escrow_info = ctx.accounts.escrow.to_account_info();
        let token_program_info = ctx.accounts.token_program.to_account_info();

        for i in 0..splits.len() {
            let amount = split_amounts[i];
            if amount == 0 {
                continue;
            }
            let to = ctx.remaining_accounts[i].clone();
            let cpi_accounts = Transfer {
                from: vault_info.clone(),
                to,
                authority: escrow_info.clone(),
            };
            let cpi_ctx = CpiContext::new_with_signer(
                token_program_info.clone(),
                cpi_accounts,
                signer_seeds,
            );
            token::transfer(cpi_ctx, amount)?;
        }

        let escrow = &mut ctx.accounts.escrow;
        escrow.status = EscrowStatus::Settled;
        escrow.settled_at = Some(Clock::get()?.unix_timestamp);
        Ok(())
    }

    /// Cancel unfunded escrow (payer only)
    pub fn cancel_escrow(ctx: Context<CancelEscrow>) -> Result<()> {
        let escrow = &ctx.accounts.escrow;
        require!(escrow.status == EscrowStatus::Created, EscrowError::InvalidStatus);
        require!(ctx.accounts.payer.key() == escrow.payer, EscrowError::Unauthorized);
        require!(ctx.accounts.vault.amount == 0, EscrowError::VaultNotEmpty);

        let escrow_id_bytes = escrow.escrow_id.to_le_bytes();
        let seeds = &[
            ESCROW_SEED,
            escrow.payer.as_ref(),
            &escrow_id_bytes,
            &[escrow.bump],
        ];
        let signer_seeds = &[&seeds[..]];

        let cpi_accounts = CloseAccount {
            account: ctx.accounts.vault.to_account_info(),
            destination: ctx.accounts.payer.to_account_info(),
            authority: ctx.accounts.escrow.to_account_info(),
        };
        let cpi_ctx = CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            cpi_accounts,
            signer_seeds,
        );
        token::close_account(cpi_ctx)?;
        Ok(())
    }

    /// Close completed escrow and reclaim rent (payer only)
    pub fn close_escrow(ctx: Context<CloseEscrow>) -> Result<()> {
        let escrow = &ctx.accounts.escrow;
        require!(
            escrow.status == EscrowStatus::Settled || escrow.status == EscrowStatus::Refunded,
            EscrowError::InvalidStatus
        );
        require!(ctx.accounts.payer.key() == escrow.payer, EscrowError::Unauthorized);
        require!(ctx.accounts.vault.amount == 0, EscrowError::VaultNotEmpty);

        let escrow_id_bytes = escrow.escrow_id.to_le_bytes();
        let seeds = &[
            ESCROW_SEED,
            escrow.payer.as_ref(),
            &escrow_id_bytes,
            &[escrow.bump],
        ];
        let signer_seeds = &[&seeds[..]];

        let cpi_accounts = CloseAccount {
            account: ctx.accounts.vault.to_account_info(),
            destination: ctx.accounts.payer.to_account_info(),
            authority: ctx.accounts.escrow.to_account_info(),
        };
        let cpi_ctx = CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            cpi_accounts,
            signer_seeds,
        );
        token::close_account(cpi_ctx)?;
        Ok(())
    }

    // ========================================================================
    // POOL ESCROW INSTRUCTIONS (Multi-Worker Tasks)
    // ========================================================================

    /// Create a pool escrow for multi-worker tasks
    pub fn create_pool_escrow(
        ctx: Context<CreatePoolEscrow>,
        escrow_id: u64,
        payment_per_worker: u64,
        max_releases: u64,
        platform_fee_bps: u16,
        release_authority: Pubkey,
        deadline: Option<i64>,
    ) -> Result<()> {
        let config = &ctx.accounts.config;
        require!(!config.paused, EscrowError::PlatformPaused);
        require!(payment_per_worker >= MIN_ESCROW_AMOUNT, EscrowError::AmountTooSmall);
        require!(max_releases >= 1 && max_releases <= MAX_POOL_WORKERS, EscrowError::InvalidMaxReleases);
        require!(platform_fee_bps <= BPS_DENOMINATOR, EscrowError::InvalidPercentage);
        require!(release_authority != Pubkey::default(), EscrowError::InvalidReleaseAuthority);

        if let Some(dl) = deadline {
            let now = Clock::get()?.unix_timestamp;
            require!(dl > now, EscrowError::DeadlineInPast);
            let max_deadline = now.checked_add(MAX_ESCROW_DURATION).ok_or(EscrowError::Overflow)?;
            require!(dl <= max_deadline, EscrowError::DeadlineTooFar);
        }

        let worker_budget = payment_per_worker
            .checked_mul(max_releases)
            .ok_or(EscrowError::Overflow)?;
        let total_fee = calculate_fee(worker_budget, platform_fee_bps)?;
        let total_funded = worker_budget.checked_add(total_fee).ok_or(EscrowError::Overflow)?;

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
        pool_escrow.platform_fee_bps = platform_fee_bps;
        pool_escrow.release_authority = release_authority;
        pool_escrow.status = PoolEscrowStatus::Created;
        pool_escrow.created_at = Clock::get()?.unix_timestamp;
        pool_escrow.funded_at = None;
        pool_escrow.closed_at = None;
        pool_escrow.deadline = deadline;
        pool_escrow.bump = ctx.bumps.pool_escrow;
        pool_escrow.vault_bump = ctx.bumps.vault;
        Ok(())
    }

    /// Fund the pool escrow with tokens
    pub fn fund_pool_escrow(ctx: Context<FundPoolEscrow>) -> Result<()> {
        let pool_escrow = &mut ctx.accounts.pool_escrow;
        require!(pool_escrow.status == PoolEscrowStatus::Created, EscrowError::InvalidStatus);
        require!(ctx.accounts.client.key() == pool_escrow.client, EscrowError::Unauthorized);

        let cpi_accounts = Transfer {
            from: ctx.accounts.client_token_account.to_account_info(),
            to: ctx.accounts.vault.to_account_info(),
            authority: ctx.accounts.client.to_account_info(),
        };
        let cpi_ctx = CpiContext::new(ctx.accounts.token_program.to_account_info(), cpi_accounts);
        token::transfer(cpi_ctx, pool_escrow.total_funded)?;

        pool_escrow.status = PoolEscrowStatus::Funded;
        pool_escrow.funded_at = Some(Clock::get()?.unix_timestamp);
        Ok(())
    }

    /// Release payment to one worker from pool
    pub fn partial_release(ctx: Context<PartialRelease>) -> Result<()> {
        let pool_escrow = &mut ctx.accounts.pool_escrow;
        require!(
            pool_escrow.status == PoolEscrowStatus::Funded || pool_escrow.status == PoolEscrowStatus::Active,
            EscrowError::InvalidStatus
        );
        require!(ctx.accounts.release_authority.key() == pool_escrow.release_authority, EscrowError::Unauthorized);

        if let Some(dl) = pool_escrow.deadline {
            require!(Clock::get()?.unix_timestamp <= dl, EscrowError::DeadlinePassed);
        }

        require!(pool_escrow.release_count < pool_escrow.max_releases, EscrowError::MaxReleasesReached);

        let worker_amount = pool_escrow.payment_per_worker;
        let platform_fee = calculate_fee(worker_amount, pool_escrow.platform_fee_bps)?;
        let total_release = worker_amount.checked_add(platform_fee).ok_or(EscrowError::Overflow)?;

        let remaining = pool_escrow
            .total_funded
            .checked_sub(pool_escrow.total_released)
            .ok_or(EscrowError::Overflow)?;
        require!(remaining >= total_release, EscrowError::InsufficientFunds);

        let escrow_id_bytes = pool_escrow.escrow_id.to_le_bytes();
        let seeds = &[
            POOL_ESCROW_SEED,
            pool_escrow.client.as_ref(),
            &escrow_id_bytes,
            &[pool_escrow.bump],
        ];
        let signer_seeds = &[&seeds[..]];

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

        pool_escrow.total_released = pool_escrow
            .total_released
            .checked_add(total_release)
            .ok_or(EscrowError::Overflow)?;
        pool_escrow.release_count = pool_escrow
            .release_count
            .checked_add(1)
            .ok_or(EscrowError::Overflow)?;
        pool_escrow.status = PoolEscrowStatus::Active;
        Ok(())
    }

    /// Close pool escrow and refund remaining funds
    pub fn close_pool_escrow(ctx: Context<ClosePoolEscrow>) -> Result<()> {
        let pool_escrow = &mut ctx.accounts.pool_escrow;
        require!(
            pool_escrow.status == PoolEscrowStatus::Funded || pool_escrow.status == PoolEscrowStatus::Active,
            EscrowError::InvalidStatus
        );
        require!(ctx.accounts.client.key() == pool_escrow.client, EscrowError::Unauthorized);

        let remaining = pool_escrow
            .total_funded
            .checked_sub(pool_escrow.total_released)
            .ok_or(EscrowError::Overflow)?;

        if remaining > 0 {
            let escrow_id_bytes = pool_escrow.escrow_id.to_le_bytes();
            let seeds = &[
                POOL_ESCROW_SEED,
                pool_escrow.client.as_ref(),
                &escrow_id_bytes,
                &[pool_escrow.bump],
            ];
            let signer_seeds = &[&seeds[..]];

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
        Ok(())
    }
}

// ============================================================================
// HELPER FUNCTIONS
// ============================================================================

fn calculate_fee(amount: u64, fee_bps: u16) -> Result<u64> {
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

fn validate_splits(splits: &[Split]) -> Result<()> {
    require!(!splits.is_empty(), EscrowError::InvalidSplits);
    require!(splits.len() <= MAX_SPLITS, EscrowError::InvalidSplits);

    let mut sum: u32 = 0;
    let mut recipients = BTreeSet::<Pubkey>::new();

    for split in splits {
        require!(split.recipient != Pubkey::default(), EscrowError::InvalidSplits);
        require!(split.bps <= BPS_DENOMINATOR, EscrowError::InvalidSplits);
        sum = sum.checked_add(split.bps as u32).ok_or(EscrowError::Overflow)?;
        require!(recipients.insert(split.recipient), EscrowError::InvalidSplits);
    }

    require!(sum == BPS_DENOMINATOR as u32, EscrowError::InvalidSplits);
    Ok(())
}

fn compute_split_amounts(total_amount: u64, splits: &[Split]) -> Result<Vec<u64>> {
    validate_splits(splits)?;
    let mut amounts = Vec::with_capacity(splits.len());
    let mut allocated: u64 = 0;

    for (i, split) in splits.iter().enumerate() {
        if i == splits.len() - 1 {
            let last = total_amount.checked_sub(allocated).ok_or(EscrowError::Overflow)?;
            amounts.push(last);
            break;
        }

        let amount = (total_amount as u128)
            .checked_mul(split.bps as u128)
            .ok_or(EscrowError::Overflow)?
            .checked_div(BPS_DENOMINATOR as u128)
            .ok_or(EscrowError::Overflow)? as u64;

        allocated = allocated.checked_add(amount).ok_or(EscrowError::Overflow)?;
        amounts.push(amount);
    }

    let sum: u64 = amounts.iter().copied().try_fold(0u64, |acc, x| acc.checked_add(x).ok_or(EscrowError::Overflow))?;
    require!(sum == total_amount, EscrowError::Overflow);
    Ok(amounts)
}

// ============================================================================
// DATA STRUCTURES
// ============================================================================

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq)]
pub struct Split {
    pub recipient: Pubkey,
    pub bps: u16,
}

impl fmt::Debug for Split {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Split")
            .field("recipient", &self.recipient)
            .field("bps", &self.bps)
            .finish()
    }
}

#[account]
pub struct PlatformConfig {
    pub admin: Pubkey,
    pub treasury: Pubkey,
    pub paused: bool,
    pub pending_admin: Option<Pubkey>,
    pub bump: u8,
}

impl PlatformConfig {
    pub const SIZE: usize = 8 + 32 + 32 + 1 + 33 + 1;
}

#[account]
pub struct EscrowAccount {
    pub escrow_id: u64,
    pub payer: Pubkey,
    pub mint: Pubkey,
    pub vault: Pubkey,
    pub total_amount: u64,
    pub splits: Vec<Split>,
    pub status: EscrowStatus,
    pub created_at: i64,
    pub funded_at: Option<i64>,
    pub approved_at: Option<i64>,
    pub settled_at: Option<i64>,
    pub refunded_at: Option<i64>,
    pub frozen_at: Option<i64>,
    pub deadline: Option<i64>,
    pub bump: u8,
    pub vault_bump: u8,
    pub version: u8,
}

impl EscrowAccount {
    pub const SIZE: usize = 8
        + 8
        + 32
        + 32
        + 32
        + 8
        + (4 + (MAX_SPLITS * (32 + 2)))
        + 1
        + 8
        + 9
        + 9
        + 9
        + 9
        + 9
        + 9
        + 1
        + 1
        + 1;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, AnchorSerialize, AnchorDeserialize)]
pub enum EscrowStatus {
    Created,
    Funded,
    Approved,
    Settled,
    Refunded,
    Frozen,
}

impl Default for EscrowStatus {
    fn default() -> Self {
        EscrowStatus::Created
    }
}

impl fmt::Display for EscrowStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EscrowStatus::Created => write!(f, "Created"),
            EscrowStatus::Funded => write!(f, "Funded"),
            EscrowStatus::Approved => write!(f, "Approved"),
            EscrowStatus::Settled => write!(f, "Settled"),
            EscrowStatus::Refunded => write!(f, "Refunded"),
            EscrowStatus::Frozen => write!(f, "Frozen"),
        }
    }
}

#[account]
pub struct PoolEscrowAccount {
    pub escrow_id: u64,
    pub client: Pubkey,
    pub mint: Pubkey,
    pub vault: Pubkey,
    pub payment_per_worker: u64,
    pub max_releases: u64,
    pub total_funded: u64,
    pub total_released: u64,
    pub release_count: u64,
    pub platform_fee_bps: u16,
    pub release_authority: Pubkey,
    pub status: PoolEscrowStatus,
    pub created_at: i64,
    pub funded_at: Option<i64>,
    pub closed_at: Option<i64>,
    pub deadline: Option<i64>,
    pub bump: u8,
    pub vault_bump: u8,
}

impl PoolEscrowAccount {
    pub const SIZE: usize = 8
        + 8
        + 32
        + 32
        + 32
        + 8
        + 8
        + 8
        + 8
        + 8
        + 2
        + 32
        + 1
        + 8
        + 9
        + 9
        + 9
        + 1
        + 1;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, AnchorSerialize, AnchorDeserialize)]
pub enum PoolEscrowStatus {
    Created,
    Funded,
    Active,
    Closed,
}

impl Default for PoolEscrowStatus {
    fn default() -> Self {
        PoolEscrowStatus::Created
    }
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
// ACCOUNT CONTEXTS
// ============================================================================

#[derive(Accounts)]
pub struct InitializeConfig<'info> {
    #[account(init, payer = admin, space = PlatformConfig::SIZE, seeds = [b"config"], bump)]
    pub config: Account<'info, PlatformConfig>,
    #[account(mut)]
    pub admin: Signer<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct UpdateConfig<'info> {
    #[account(mut, seeds = [b"config"], bump = config.bump, has_one = admin @ EscrowError::Unauthorized)]
    pub config: Account<'info, PlatformConfig>,
    pub admin: Signer<'info>,
}

#[derive(Accounts)]
pub struct ProposeAdmin<'info> {
    #[account(mut, seeds = [b"config"], bump = config.bump, has_one = admin @ EscrowError::Unauthorized)]
    pub config: Account<'info, PlatformConfig>,
    pub admin: Signer<'info>,
}

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
    #[account(
        init,
        payer = payer,
        space = EscrowAccount::SIZE,
        seeds = [ESCROW_SEED, payer.key().as_ref(), &escrow_id.to_le_bytes()],
        bump
    )]
    pub escrow: Account<'info, EscrowAccount>,
    #[account(
        init,
        payer = payer,
        token::mint = mint,
        token::authority = escrow,
        seeds = [VAULT_SEED, escrow.key().as_ref()],
        bump
    )]
    pub vault: Account<'info, TokenAccount>,
    pub mint: Account<'info, Mint>,
    #[account(mut)]
    pub payer: Signer<'info>,
    pub system_program: Program<'info, System>,
    #[account(address = token::ID)]
    pub token_program: Program<'info, Token>,
    pub rent: Sysvar<'info, Rent>,
}

#[derive(Accounts)]
pub struct FundEscrow<'info> {
    #[account(
        mut,
        seeds = [ESCROW_SEED, escrow.payer.as_ref(), &escrow.escrow_id.to_le_bytes()],
        bump = escrow.bump,
        has_one = payer @ EscrowError::Unauthorized,
        has_one = vault @ EscrowError::InvalidVault,
        has_one = mint @ EscrowError::InvalidMint
    )]
    pub escrow: Account<'info, EscrowAccount>,
    #[account(mut, seeds = [VAULT_SEED, escrow.key().as_ref()], bump = escrow.vault_bump)]
    pub vault: Account<'info, TokenAccount>,
    #[account(mut, token::mint = mint, token::authority = payer)]
    pub payer_token_account: Account<'info, TokenAccount>,
    pub mint: Account<'info, Mint>,
    pub payer: Signer<'info>,
    #[account(address = token::ID)]
    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct ApproveEscrow<'info> {
    #[account(
        mut,
        seeds = [ESCROW_SEED, escrow.payer.as_ref(), &escrow.escrow_id.to_le_bytes()],
        bump = escrow.bump,
        has_one = payer @ EscrowError::Unauthorized
    )]
    pub escrow: Account<'info, EscrowAccount>,
    pub payer: Signer<'info>,
}

#[derive(Accounts)]
pub struct SettleEscrow<'info> {
    #[account(
        mut,
        seeds = [ESCROW_SEED, escrow.payer.as_ref(), &escrow.escrow_id.to_le_bytes()],
        bump = escrow.bump,
        has_one = payer @ EscrowError::Unauthorized,
        has_one = vault @ EscrowError::InvalidVault
    )]
    pub escrow: Account<'info, EscrowAccount>,
    #[account(mut, seeds = [VAULT_SEED, escrow.key().as_ref()], bump = escrow.vault_bump)]
    pub vault: Account<'info, TokenAccount>,
    pub payer: Signer<'info>,
    #[account(address = token::ID)]
    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct RefundEscrow<'info> {
    #[account(
        mut,
        seeds = [ESCROW_SEED, escrow.payer.as_ref(), &escrow.escrow_id.to_le_bytes()],
        bump = escrow.bump,
        has_one = payer @ EscrowError::Unauthorized,
        has_one = vault @ EscrowError::InvalidVault
    )]
    pub escrow: Account<'info, EscrowAccount>,
    #[account(mut, seeds = [VAULT_SEED, escrow.key().as_ref()], bump = escrow.vault_bump)]
    pub vault: Account<'info, TokenAccount>,
    #[account(mut, token::mint = escrow.mint, token::authority = payer)]
    pub payer_token_account: Account<'info, TokenAccount>,
    pub payer: Signer<'info>,
    #[account(address = token::ID)]
    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct FreezeEscrow<'info> {
    #[account(seeds = [b"config"], bump = config.bump)]
    pub config: Account<'info, PlatformConfig>,
    #[account(
        mut,
        seeds = [ESCROW_SEED, escrow.payer.as_ref(), &escrow.escrow_id.to_le_bytes()],
        bump = escrow.bump
    )]
    pub escrow: Account<'info, EscrowAccount>,
    pub caller: Signer<'info>,
}

#[derive(Accounts)]
pub struct AdminRefundToPayer<'info> {
    #[account(seeds = [b"config"], bump = config.bump, has_one = admin @ EscrowError::Unauthorized)]
    pub config: Account<'info, PlatformConfig>,
    #[account(
        mut,
        seeds = [ESCROW_SEED, escrow.payer.as_ref(), &escrow.escrow_id.to_le_bytes()],
        bump = escrow.bump,
        has_one = vault @ EscrowError::InvalidVault
    )]
    pub escrow: Account<'info, EscrowAccount>,
    #[account(mut, seeds = [VAULT_SEED, escrow.key().as_ref()], bump = escrow.vault_bump)]
    pub vault: Account<'info, TokenAccount>,
    #[account(mut, token::mint = escrow.mint, token::authority = escrow.payer)]
    pub payer_token_account: Account<'info, TokenAccount>,
    pub admin: Signer<'info>,
    #[account(address = token::ID)]
    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct AdminSettleEscrow<'info> {
    #[account(seeds = [b"config"], bump = config.bump, has_one = admin @ EscrowError::Unauthorized)]
    pub config: Account<'info, PlatformConfig>,
    #[account(
        mut,
        seeds = [ESCROW_SEED, escrow.payer.as_ref(), &escrow.escrow_id.to_le_bytes()],
        bump = escrow.bump,
        has_one = vault @ EscrowError::InvalidVault
    )]
    pub escrow: Account<'info, EscrowAccount>,
    #[account(mut, seeds = [VAULT_SEED, escrow.key().as_ref()], bump = escrow.vault_bump)]
    pub vault: Account<'info, TokenAccount>,
    pub admin: Signer<'info>,
    #[account(address = token::ID)]
    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct CancelEscrow<'info> {
    #[account(
        mut,
        seeds = [ESCROW_SEED, escrow.payer.as_ref(), &escrow.escrow_id.to_le_bytes()],
        bump = escrow.bump,
        has_one = payer @ EscrowError::Unauthorized,
        close = payer
    )]
    pub escrow: Account<'info, EscrowAccount>,
    #[account(mut, seeds = [VAULT_SEED, escrow.key().as_ref()], bump = escrow.vault_bump)]
    pub vault: Account<'info, TokenAccount>,
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(address = token::ID)]
    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct CloseEscrow<'info> {
    #[account(
        mut,
        seeds = [ESCROW_SEED, escrow.payer.as_ref(), &escrow.escrow_id.to_le_bytes()],
        bump = escrow.bump,
        has_one = payer @ EscrowError::Unauthorized,
        close = payer
    )]
    pub escrow: Account<'info, EscrowAccount>,
    #[account(mut, seeds = [VAULT_SEED, escrow.key().as_ref()], bump = escrow.vault_bump)]
    pub vault: Account<'info, TokenAccount>,
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(address = token::ID)]
    pub token_program: Program<'info, Token>,
}

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
    #[account(seeds = [b"config"], bump = config.bump)]
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
    #[account(
        mut,
        token::mint = pool_escrow.mint,
        constraint = worker_token_account.key() != treasury_token_account.key() @ EscrowError::DuplicateAccounts
    )]
    pub worker_token_account: Account<'info, TokenAccount>,
    #[account(
        mut,
        token::mint = pool_escrow.mint,
        constraint = treasury_token_account.owner == config.treasury @ EscrowError::InvalidTreasury
    )]
    pub treasury_token_account: Account<'info, TokenAccount>,
    pub release_authority: Signer<'info>,
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
        close = client
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
// ERROR CODES
// ============================================================================

#[error_code]
pub enum EscrowError {
    #[msg("Invalid escrow status for this operation")]
    InvalidStatus,
    #[msg("Unauthorized")]
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
    #[msg("Invalid treasury address")]
    InvalidTreasury,
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
    #[msg("Duplicate mutable accounts not allowed")]
    DuplicateAccounts,
    #[msg("Invalid max_releases value (must be 1-10000)")]
    InvalidMaxReleases,
    #[msg("Maximum number of releases reached")]
    MaxReleasesReached,
    #[msg("Invalid release authority address")]
    InvalidReleaseAuthority,
    #[msg("Deadline has passed - no more releases allowed")]
    DeadlinePassed,
    #[msg("Invalid splits")]
    InvalidSplits,
    #[msg("Invalid recipient token account")]
    InvalidRecipientTokenAccount,
    #[msg("Invalid number of remaining accounts")]
    InvalidRemainingAccounts,
}

// ============================================================================
// PROPERTY-BASED TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn split_amounts_sum_to_total(total_amount in 1u64..=1_000_000_000u64, bps_a in 0u16..=10_000u16) {
            let bps_b = (BPS_DENOMINATOR as u32).saturating_sub(bps_a as u32) as u16;
            let a = Split { recipient: Pubkey::new_unique(), bps: bps_a };
            let b = Split { recipient: Pubkey::new_unique(), bps: bps_b };
            let splits = vec![a, b];
            let amounts = compute_split_amounts(total_amount, &splits).unwrap();
            prop_assert_eq!(amounts.len(), 2);
            prop_assert_eq!(amounts[0].saturating_add(amounts[1]), total_amount);
        }
    }

    #[test]
    fn validate_splits_rejects_duplicates() {
        let recipient = Pubkey::new_unique();
        let splits = vec![
            Split { recipient, bps: 5000 },
            Split { recipient, bps: 5000 },
        ];
        assert!(validate_splits(&splits).is_err());
    }

    #[test]
    fn validate_splits_rejects_wrong_sum() {
        let splits = vec![
            Split { recipient: Pubkey::new_unique(), bps: 4000 },
            Split { recipient: Pubkey::new_unique(), bps: 5000 },
        ];
        assert!(validate_splits(&splits).is_err());
    }

    proptest! {
        #[test]
        fn fee_never_exceeds_amount(amount in 1u64..=u64::MAX/2, fee_bps in 0u16..=10_000u16) {
            let fee = calculate_fee(amount, fee_bps).unwrap();
            prop_assert!(fee <= amount);
        }
    }

    // Additional unit tests for split validation
    #[test]
    fn validate_splits_accepts_valid_2way() {
        let splits = vec![
            Split { recipient: Pubkey::new_unique(), bps: 9000 },
            Split { recipient: Pubkey::new_unique(), bps: 1000 },
        ];
        assert!(validate_splits(&splits).is_ok());
    }

    #[test]
    fn validate_splits_accepts_valid_3way() {
        let splits = vec![
            Split { recipient: Pubkey::new_unique(), bps: 9000 },
            Split { recipient: Pubkey::new_unique(), bps: 700 },
            Split { recipient: Pubkey::new_unique(), bps: 300 },
        ];
        assert!(validate_splits(&splits).is_ok());
    }

    #[test]
    fn validate_splits_rejects_empty() {
        let splits: Vec<Split> = vec![];
        assert!(validate_splits(&splits).is_err());
    }

    #[test]
    fn validate_splits_rejects_zero_address() {
        let splits = vec![
            Split { recipient: Pubkey::default(), bps: 5000 },
            Split { recipient: Pubkey::new_unique(), bps: 5000 },
        ];
        assert!(validate_splits(&splits).is_err());
    }

    #[test]
    fn validate_splits_rejects_too_many() {
        let mut splits = Vec::new();
        for _ in 0..=MAX_SPLITS {
            splits.push(Split { recipient: Pubkey::new_unique(), bps: 1000 });
        }
        assert!(validate_splits(&splits).is_err());
    }

    // Unit tests for split amount calculation
    #[test]
    fn compute_split_amounts_2way() {
        let total = 100_000_000; // 100 USDC (6 decimals)
        let splits = vec![
            Split { recipient: Pubkey::new_unique(), bps: 9000 },
            Split { recipient: Pubkey::new_unique(), bps: 1000 },
        ];
        let amounts = compute_split_amounts(total, &splits).unwrap();
        assert_eq!(amounts.len(), 2);
        assert_eq!(amounts[0], 90_000_000); // 90 USDC
        assert_eq!(amounts[1], 10_000_000); // 10 USDC
        assert_eq!(amounts[0] + amounts[1], total);
    }

    #[test]
    fn compute_split_amounts_3way() {
        let total = 100_000_000; // 100 USDC
        let splits = vec![
            Split { recipient: Pubkey::new_unique(), bps: 9000 },
            Split { recipient: Pubkey::new_unique(), bps: 700 },
            Split { recipient: Pubkey::new_unique(), bps: 300 },
        ];
        let amounts = compute_split_amounts(total, &splits).unwrap();
        assert_eq!(amounts.len(), 3);
        assert_eq!(amounts[0], 90_000_000); // 90 USDC
        assert_eq!(amounts[1], 7_000_000);  // 7 USDC
        assert_eq!(amounts[2], 3_000_000);  // 3 USDC
        assert_eq!(amounts[0] + amounts[1] + amounts[2], total);
    }

    #[test]
    fn compute_split_amounts_handles_rounding() {
        let total = 100; // Small amount to test rounding
        let splits = vec![
            Split { recipient: Pubkey::new_unique(), bps: 3333 },
            Split { recipient: Pubkey::new_unique(), bps: 3333 },
            Split { recipient: Pubkey::new_unique(), bps: 3334 },
        ];
        let amounts = compute_split_amounts(total, &splits).unwrap();
        assert_eq!(amounts.len(), 3);
        // Verify sum equals total (remainder goes to last recipient)
        assert_eq!(amounts[0] + amounts[1] + amounts[2], total);
    }

    #[test]
    fn compute_split_amounts_large_amount() {
        let total = 1_000_000_000_000; // 1 million USDC
        let splits = vec![
            Split { recipient: Pubkey::new_unique(), bps: 9500 },
            Split { recipient: Pubkey::new_unique(), bps: 500 },
        ];
        let amounts = compute_split_amounts(total, &splits).unwrap();
        assert_eq!(amounts.len(), 2);
        assert_eq!(amounts[0], 950_000_000_000);
        assert_eq!(amounts[1], 50_000_000_000);
        assert_eq!(amounts[0] + amounts[1], total);
    }

    // Additional property-based tests
    proptest! {
        #[test]
        fn prop_no_split_exceeds_total(
            total_amount in 1u64..=1_000_000_000u64,
            bps in 1u16..=10_000u16
        ) {
            let splits = vec![
                Split { recipient: Pubkey::new_unique(), bps },
                Split { recipient: Pubkey::new_unique(), bps: BPS_DENOMINATOR - bps },
            ];
            let amounts = compute_split_amounts(total_amount, &splits).unwrap();
            for amount in amounts {
                prop_assert!(amount <= total_amount);
            }
        }

        #[test]
        fn prop_calculation_is_deterministic(
            total_amount in 1u64..=1_000_000_000u64,
            bps in 1u16..=9999u16
        ) {
            let splits = vec![
                Split { recipient: Pubkey::new_unique(), bps },
                Split { recipient: Pubkey::new_unique(), bps: BPS_DENOMINATOR - bps },
            ];
            let amounts1 = compute_split_amounts(total_amount, &splits).unwrap();
            let amounts2 = compute_split_amounts(total_amount, &splits).unwrap();
            prop_assert_eq!(amounts1, amounts2);
        }

        #[test]
        fn prop_3way_split_sums_to_total(
            total_amount in 1u64..=1_000_000_000u64,
            bps1 in 1u16..=8000u16,
            bps2 in 1u16..=1000u16
        ) {
            let bps3 = BPS_DENOMINATOR.saturating_sub(bps1).saturating_sub(bps2);
            if bps3 > 0 && bps1 + bps2 + bps3 == BPS_DENOMINATOR {
                let splits = vec![
                    Split { recipient: Pubkey::new_unique(), bps: bps1 },
                    Split { recipient: Pubkey::new_unique(), bps: bps2 },
                    Split { recipient: Pubkey::new_unique(), bps: bps3 },
                ];
                let amounts = compute_split_amounts(total_amount, &splits).unwrap();
                let sum: u64 = amounts.iter().sum();
                prop_assert_eq!(sum, total_amount);
            }
        }
    }
}
