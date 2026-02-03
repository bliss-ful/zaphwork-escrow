# ZaphWork V2 Architecture: Infrastructure Primitive Design

## Executive Summary

ZaphWork V2 transforms from an **application-specific backend** into a **reusable infrastructure primitive** for the Solana ecosystem. This document explains the architectural decisions, design patterns, and positioning for grant applications.

---

## Table of Contents

1. [The Problem with V1](#the-problem-with-v1)
2. [The V2 Solution](#the-v2-solution)
3. [Split-Based Settlement Design](#split-based-settlement-design)
4. [Core Protocol vs Application Layer](#core-protocol-vs-application-layer)
5. [Use Cases Beyond Freelancing](#use-cases-beyond-freelancing)
6. [Security Properties](#security-properties)
7. [Integration Examples](#integration-examples)

---

## The Problem with V1

### Hardcoded Business Logic

```rust
// V1: Hardcoded fees - cannot be changed
pub const DEFAULT_FEE_BPS: u64 = 1000;        // 10% for tasks
pub const EMPLOYMENT_FEE_BPS: u64 = 500;      // 5% for jobs
pub const EXPECTED_ADMIN: &str = "HDz6...";   // Single admin

// V1: Application-specific types
pub enum EscrowType {
    Task,        // Freelance work
    Employment,  // Long-term contracts
}
```

**Problems:**
- ❌ Fees baked into the protocol
- ❌ Only supports 2-party escrows (client → worker)
- ❌ Cannot be reused by other applications
- ❌ Tied to ZaphWork's business model

### Limited Flexibility

```rust
// V1: Single worker, single payment
pub struct EscrowAccount {
    pub client: Pubkey,
    pub worker: Pubkey,           // Only one recipient
    pub worker_amount: u64,
    pub platform_fee_bps: u64,    // Hardcoded logic
    pub escrow_type: EscrowType,  // Application-specific
}
```

---

## The V2 Solution

### Generic Split-Based Design

```rust
// V2: No hardcoded fees
pub const MIN_ESCROW_AMOUNT: u64 = 1_000_000;  // Safety minimum
pub const BPS_DENOMINATOR: u16 = 10_000;       // Standard basis points
pub const MAX_SPLITS: usize = 8;               // Reasonable limit

// V2: Generic split structure
pub struct Split {
    pub recipient: Pubkey,  // Any recipient
    pub bps: u16,           // Percentage in basis points
}
```

**Benefits:**
- ✅ Caller defines fee structure
- ✅ Supports multi-party settlements (up to 8 recipients)
- ✅ Reusable by any application
- ✅ No business logic in protocol

### Flexible Escrow Account

```rust
// V2: Generic escrow for any use case
pub struct EscrowAccount {
    pub payer: Pubkey,              // More generic than "client"
    pub splits: Vec<Split>,         // Multiple recipients
    pub total_amount: u64,          // Clear semantics
    pub version: u8,                // Future-proof
    // No escrow_type, no platform_fee_bps
}
```

---

## Split-Based Settlement Design

### How It Works

1. **Caller Defines Splits**
   ```rust
   let splits = vec![
       Split { recipient: worker, bps: 9000 },    // 90%
       Split { recipient: platform, bps: 1000 },  // 10%
   ];
   ```

2. **Protocol Validates**
   ```rust
   fn validate_splits(splits: &[Split]) -> Result<()> {
       // Must not be empty
       require!(!splits.is_empty());
       
       // Must not exceed MAX_SPLITS
       require!(splits.len() <= MAX_SPLITS);
       
       // No duplicate recipients
       let mut seen = BTreeSet::new();
       for split in splits {
           require!(seen.insert(split.recipient));
       }
       
       // BPS must sum to exactly 10,000
       let sum: u32 = splits.iter().map(|s| s.bps as u32).sum();
       require!(sum == BPS_DENOMINATOR as u32);
       
       Ok(())
   }
   ```

3. **Protocol Calculates Amounts**
   ```rust
   fn compute_split_amounts(total: u64, splits: &[Split]) -> Result<Vec<u64>> {
       let mut amounts = Vec::new();
       let mut allocated = 0u64;
       
       for (i, split) in splits.iter().enumerate() {
           if i == splits.len() - 1 {
               // Last recipient gets remainder (prevents rounding errors)
               amounts.push(total - allocated);
           } else {
               let amount = (total as u128 * split.bps as u128 / 10000) as u64;
               amounts.push(amount);
               allocated += amount;
           }
       }
       
       Ok(amounts)
   }
   ```

4. **Protocol Distributes**
   ```rust
   // Transfer to each recipient in order
   for (i, amount) in split_amounts.iter().enumerate() {
       token::transfer(
           ctx,
           *amount,
           remaining_accounts[i]  // Token account for split.recipient
       )?;
   }
   ```

### Why This Design?

**Correctness:**
- Amounts always sum to total (last recipient gets remainder)
- No rounding errors or lost tokens
- Property-based tests verify invariants

**Flexibility:**
- Any number of recipients (up to 8)
- Any percentage distribution
- Caller controls fee structure

**Simplicity:**
- Protocol doesn't care about "fees" vs "payments"
- Just distributes according to splits
- No business logic

---

## Core Protocol vs Application Layer

### Separation of Concerns

```
┌─────────────────────────────────────────────────────────┐
│                  APPLICATION LAYER                       │
│  (ZaphWork, DAOs, Bounty Platforms, etc.)               │
│                                                          │
│  - Business logic (what is a "fee"?)                    │
│  - User interface                                        │
│  - Fee calculation                                       │
│  - Split generation                                      │
└─────────────────────────────────────────────────────────┘
                           │
                           │ Calls with splits
                           ▼
┌─────────────────────────────────────────────────────────┐
│                   CORE PROTOCOL                          │
│  (ZaphWork Smart Contract V2)                           │
│                                                          │
│  - Escrow creation                                       │
│  - Fund custody                                          │
│  - Split validation                                      │
│  - Atomic distribution                                   │
│  - Dispute resolution                                    │
└─────────────────────────────────────────────────────────┘
```

### Example: ZaphWork Application Layer

```typescript
// Application decides fee structure
const TASK_FEE_BPS = 1000;  // 10%
const JOB_FEE_BPS = 500;    // 5%

function createTaskEscrow(worker: PublicKey, amount: number) {
  // Application calculates splits
  const splits = [
    { recipient: worker, bps: 10000 - TASK_FEE_BPS },
    { recipient: TREASURY, bps: TASK_FEE_BPS }
  ];
  
  // Protocol just executes
  return program.methods.createEscrow(
    escrowId,
    amount,
    splits,  // Application-defined
    deadline
  );
}
```

### Example: DAO Treasury

```typescript
// DAO decides custom split
function createProposalPayout(recipients: Recipient[]) {
  const splits = recipients.map(r => ({
    recipient: r.pubkey,
    bps: r.percentage * 100  // Convert % to bps
  }));
  
  // Same protocol, different use case
  return program.methods.createEscrow(
    proposalId,
    totalBudget,
    splits,  // DAO-defined
    deadline
  );
}
```

---

## Use Cases Beyond Freelancing

### 1. DAO Treasury Management

```rust
// Multi-sig with custom splits
let splits = vec![
    Split { recipient: core_team, bps: 5000 },      // 50%
    Split { recipient: marketing, bps: 2000 },      // 20%
    Split { recipient: development, bps: 2000 },    // 20%
    Split { recipient: reserve, bps: 1000 },        // 10%
];
```

### 2. Bounty Platforms

```rust
// Flexible reward distribution
let splits = vec![
    Split { recipient: winner, bps: 7000 },         // 70%
    Split { recipient: runner_up, bps: 2000 },      // 20%
    Split { recipient: platform, bps: 1000 },       // 10%
];
```

### 3. Revenue Sharing

```rust
// Content creator + affiliates
let splits = vec![
    Split { recipient: creator, bps: 7000 },        // 70%
    Split { recipient: affiliate1, bps: 1500 },     // 15%
    Split { recipient: affiliate2, bps: 1000 },     // 10%
    Split { recipient: platform, bps: 500 },        // 5%
];
```

### 4. Payroll with Tax Withholding

```rust
// Salary split
let splits = vec![
    Split { recipient: employee, bps: 7500 },       // 75% net
    Split { recipient: tax_authority, bps: 2000 },  // 20% tax
    Split { recipient: benefits, bps: 500 },        // 5% benefits
];
```

### 5. Crowdfunding Milestones

```rust
// Milestone-based release
let splits = vec![
    Split { recipient: project, bps: 9500 },        // 95% to project
    Split { recipient: platform, bps: 500 },        // 5% platform fee
];
```

---

## Security Properties

### 1. Arithmetic Safety

```rust
// All operations use checked arithmetic
let total = worker_amount
    .checked_add(platform_fee)
    .ok_or(EscrowError::Overflow)?;
```

### 2. No Duplicate Recipients

```rust
// BTreeSet prevents duplicates
let mut recipients = BTreeSet::new();
for split in splits {
    require!(recipients.insert(split.recipient));
}
```

### 3. Exact Amount Distribution

```rust
// Property: sum of splits == total_amount
let sum: u64 = amounts.iter().sum();
require!(sum == total_amount);
```

### 4. Authorization Checks

```rust
// Only payer can settle
require!(ctx.accounts.payer.key() == escrow.payer);

// Only admin can resolve disputes
require!(ctx.accounts.admin.key() == config.admin);
```

### 5. PDA Security

```rust
// Deterministic account derivation
seeds = [ESCROW_SEED, payer, escrow_id]
// Prevents account confusion attacks
```

---

## Integration Examples

### Basic 2-Party Escrow

```typescript
// Simple freelance payment
const splits = [
  { recipient: worker, bps: 9000 },   // 90%
  { recipient: platform, bps: 1000 }  // 10%
];

await program.methods.createEscrow(
  escrowId,
  1_100_000,  // 1.1 USDC total
  splits,
  deadline
).rpc();
```

### Multi-Party Settlement

```typescript
// Revenue sharing with 4 parties
const splits = [
  { recipient: creator, bps: 6000 },    // 60%
  { recipient: partner1, bps: 2000 },   // 20%
  { recipient: partner2, bps: 1500 },   // 15%
  { recipient: platform, bps: 500 }     // 5%
];

// Get token accounts for all recipients
const remainingAccounts = await Promise.all(
  splits.map(s => getAssociatedTokenAddress(mint, s.recipient))
);

await program.methods.settleEscrow()
  .remainingAccounts(remainingAccounts.map(pubkey => ({
    pubkey,
    isSigner: false,
    isWritable: true
  })))
  .rpc();
```

### DAO Proposal Payout

```typescript
// DAO votes on split percentages
const proposalSplits = await dao.getApprovedSplits(proposalId);

await program.methods.createEscrow(
  proposalId,
  dao.budget,
  proposalSplits,
  null  // No deadline for DAO payouts
).rpc();
```

---

## Property-Based Testing

### Key Properties Verified

1. **Split Amounts Sum to Total**
   ```rust
   proptest! {
       fn split_amounts_sum_to_total(
           total_amount in 1u64..=1_000_000_000u64,
           bps_a in 0u16..=10_000u16
       ) {
           let bps_b = 10_000 - bps_a;
           let splits = vec![
               Split { recipient: Pubkey::new_unique(), bps: bps_a },
               Split { recipient: Pubkey::new_unique(), bps: bps_b },
           ];
           let amounts = compute_split_amounts(total_amount, &splits)?;
           assert_eq!(amounts[0] + amounts[1], total_amount);
       }
   }
   ```

2. **Fee Never Exceeds Amount**
   ```rust
   proptest! {
       fn fee_never_exceeds_amount(
           amount in 1u64..=u64::MAX/2,
           fee_bps in 0u16..=10_000u16
       ) {
           let fee = calculate_fee(amount, fee_bps)?;
           assert!(fee <= amount);
       }
   }
   ```

3. **Validation Rejects Invalid Splits**
   ```rust
   #[test]
   fn validate_splits_rejects_duplicates() {
       let recipient = Pubkey::new_unique();
       let splits = vec![
           Split { recipient, bps: 5000 },
           Split { recipient, bps: 5000 },  // Duplicate!
       ];
       assert!(validate_splits(&splits).is_err());
   }
   ```

---

## Grant Positioning

### Why This Matters for Grants

**Solana Foundation:**
- Infrastructure that benefits the entire ecosystem
- Not tied to a single application
- Demonstrates technical excellence
- Property-based testing shows rigor

**Circle:**
- USDC-native settlement primitive
- Enables real economic activity
- Multiple use cases for stablecoin payments
- Instant atomic settlements

**Superteam:**
- Real-world adoption in target regions
- Enables local payment infrastructure
- Community-driven development
- Open-source for ecosystem growth

---

## Conclusion

ZaphWork V2 is not just a freelance platform backend—it's a **reusable escrow primitive** for the Solana ecosystem. By removing hardcoded business logic and implementing generic split-based settlement, we've created infrastructure that can power:

- Freelance platforms
- DAO treasuries
- Bounty systems
- Revenue sharing
- Payroll services
- Crowdfunding
- And more...

This positions ZaphWork as **infrastructure** rather than just an application, making it grant-worthy and valuable to the broader Solana ecosystem.

---

*For implementation details, see [DEPLOYMENT_INFO_V2.md](./DEPLOYMENT_INFO_V2.md)*
