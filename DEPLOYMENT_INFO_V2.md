# ZaphWork Smart Contract V2 - Deployment Information

## Deployment Status: ✅ LIVE ON DEVNET

**Deployment Date:** February 3, 2026  
**Version:** 2.0.0 (Infrastructure Primitive)  
**Network:** Solana Devnet

---

## Program Information

- **Program ID:** `3iKABSF5zoQjGPykxUQNxbm7eQADqs8DreuGupggc679`
- **IDL Account:** `EWzD6cSHUzcRbHWsTxMsj4WUPpLc8vEMo5KJqeeTqctE`
- **Upgrade Authority:** `HDz6adAXLsReVUigJbkFn5rNkJdAyyECqfxyAtypa28S`
- **Program Data Address:** `9rL81ttN9xdpm1xdgR4ipV5LyZYnCmciNZU28wtT9PVa`
- **Deployment Slot:** 439650576
- **Program Size:** 560,672 bytes (547 KB)
- **Rent Balance:** 3.9034812 SOL

---

## Key Changes from V1

### Architecture Transformation
**From:** Application Backend → **To:** Infrastructure Primitive

### Removed Hardcoded Business Logic
- ❌ `DEFAULT_FEE_BPS` (10% hardcoded fee)
- ❌ `EMPLOYMENT_FEE_BPS` (5% hardcoded fee)
- ❌ `EscrowType` enum (Task vs Employment)
- ❌ `EXPECTED_ADMIN` constant
- ❌ `platform_authority` field from PlatformConfig

### New Split-Based Settlement
- ✅ Generic `Split` struct with `recipient: Pubkey` and `bps: u16`
- ✅ Support for up to 8 split recipients (`MAX_SPLITS`)
- ✅ Caller-defined fee distribution
- ✅ Flexible payment splitting for any use case

### Updated Data Structures
- **EscrowAccount:**
  - `client` → `payer` (more generic naming)
  - `worker: Pubkey` → `splits: Vec<Split>` (multi-recipient)
  - `worker_amount: u64` → `total_amount: u64` (clearer semantics)
  - Removed `platform_fee_bps` and `escrow_type` fields
  - Added `version: u8` field (set to 2 for new escrows)
  - Added `approved_at` and `settled_at` timestamps

- **PlatformConfig:**
  - Removed `platform_authority` field
  - Simplified to just `admin`, `treasury`, `paused`, `pending_admin`

### Updated PDA Seeds
- **V1:** `[ESCROW_SEED, client, worker, escrow_id]`
- **V2:** `[ESCROW_SEED, payer, escrow_id]`

---

## New Instructions

### Core Escrow (Updated)
1. `create_escrow(escrow_id, total_amount, splits, deadline)` - Now accepts splits array
2. `fund_escrow()` - Unchanged
3. `approve_escrow()` - New optional step before settlement
4. `settle_escrow()` - Now distributes to multiple recipients via remaining accounts
5. `refund_escrow()` - Unchanged
6. `cancel_escrow()` - Unchanged
7. `close_escrow()` - Unchanged

### Dispute Layer (Updated)
8. `freeze_escrow()` - Now checks if caller is in splits array
9. `admin_refund_to_payer()` - Unchanged
10. `admin_settle_with_splits(splits)` - Admin can override splits for dispute resolution

### Pool Escrow (Updated)
11. `create_pool_escrow(..., release_authority)` - Now accepts per-pool authority
12. `fund_pool_escrow()` - Unchanged
13. `partial_release()` - Now uses pool-specific release_authority
14. `close_pool_escrow()` - Unchanged

---

## Property-Based Testing

All 97 tests pass:
- ✅ 5 unit tests (split validation, fee calculation)
- ✅ 36 property-based tests (QuickCheck-style)
- ✅ 56 integration tests (full escrow lifecycle)

Key properties verified:
- Split amounts always sum to total_amount
- Fees never exceed the base amount
- No duplicate recipients allowed
- BPS must sum to exactly 10,000

---

## Migration Guide

### For Frontend Developers

**Old API (V1):**
```typescript
await program.methods.createEscrow(
  escrowId,
  workerAmount,  // What worker receives
  escrowType,    // Task or Employment
  deadline
)
```

**New API (V2):**
```typescript
const splits = [
  { recipient: workerPubkey, bps: 9000 },    // 90% to worker
  { recipient: platformPubkey, bps: 1000 }   // 10% to platform
];

await program.methods.createEscrow(
  escrowId,
  totalAmount,   // Total including all splits
  splits,        // Array of recipients and percentages
  deadline
)
```

### Calculating Splits

```typescript
function calculateSplits(
  workerPubkey: PublicKey,
  platformPubkey: PublicKey,
  workerAmount: number,
  platformFeeBps: number
): Split[] {
  const workerBps = 10000 - platformFeeBps;
  return [
    { recipient: workerPubkey, bps: workerBps },
    { recipient: platformPubkey, bps: platformFeeBps }
  ];
}

// Example: 90/10 split
const splits = calculateSplits(
  worker,
  treasury,
  1_000_000,  // 1 USDC to worker
  1000        // 10% fee (1000 bps)
);
// Total amount = 1_000_000 + 111_111 = 1_111_111
```

### Settlement with Remaining Accounts

```typescript
// Get token accounts for each split recipient
const remainingAccounts = await Promise.all(
  splits.map(async (split) => ({
    pubkey: await getAssociatedTokenAddress(mint, split.recipient),
    isSigner: false,
    isWritable: true
  }))
);

await program.methods.settleEscrow()
  .accounts({ /* ... */ })
  .remainingAccounts(remainingAccounts)
  .rpc();
```

---

## Grant Positioning

This refactoring positions ZaphWork as an **infrastructure primitive** suitable for:

### Solana Foundation Grant
- ✅ Reusable escrow protocol for the ecosystem
- ✅ No hardcoded business logic
- ✅ Property-based testing demonstrates correctness
- ✅ Clean, well-documented code

### Circle Developer Grant
- ✅ USDC-native settlement layer
- ✅ Instant atomic payments
- ✅ Multiple use cases beyond freelancing
- ✅ Real economic activity on Solana

### Superteam Grant
- ✅ Real-world adoption in LATAM/Asia
- ✅ Enables local payment infrastructure
- ✅ Community-driven development
- ✅ Open-source infrastructure

---

## Use Cases Beyond Freelancing

The generic split-based design enables:

1. **DAO Treasury Management** - Multi-sig with custom splits
2. **Bounty Platforms** - Flexible reward distribution
3. **Revenue Sharing** - Content creators, affiliates, partners
4. **Escrow Services** - Real estate, e-commerce, P2P trades
5. **Payroll Systems** - Salary splits, tax withholding
6. **Crowdfunding** - Milestone-based fund releases
7. **Subscription Services** - Recurring payments with splits

---

## Next Steps

### Immediate (This Week)
- [ ] Update frontend to use new split-based API
- [ ] Update E2E tests for new interface
- [ ] Create example integration code
- [ ] Write ARCHITECTURE.md documentation

### Short-term (Next 2 Weeks)
- [ ] Submit Solana Foundation grant application
- [ ] Submit Circle Developer Grant application
- [ ] Submit Superteam grant application
- [ ] Create developer documentation site

### Medium-term (Next Month)
- [ ] Deploy to mainnet after thorough testing
- [ ] Publish SDK for easy integration
- [ ] Create video tutorials
- [ ] Onboard first external integrators

---

## Security Considerations

- ✅ All arithmetic uses checked operations (no overflow)
- ✅ PDA derivation prevents account confusion
- ✅ Duplicate account checks prevent double-spending
- ✅ Authorization checks on all sensitive operations
- ✅ Property-based tests verify invariants
- ✅ No hardcoded addresses (except program ID)

---

## Support & Resources

- **Documentation:** [Coming Soon]
- **GitHub:** https://github.com/yourusername/zaphwork
- **Discord:** [Coming Soon]
- **Email:** support@zaphwork.com

---

*Last Updated: February 3, 2026*
