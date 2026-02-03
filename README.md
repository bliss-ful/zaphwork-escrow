# ZaphWork Escrow Smart Contract

A secure, trustless escrow smart contract for freelance payments on Solana. Built with [Anchor](https://www.anchor-lang.com/).

## Features

- **PDA-based Escrows** - Each escrow is a Program Derived Address for security
- **Split-Based Settlement** - Flexible payment distribution to multiple recipients (up to 8)
- **Two-Step Admin Transfer** - Secure admin role transfer with propose/accept pattern
- **Dispute Resolution** - Freeze, admin release, admin refund, and split funds
- **Pool Escrows** - Multi-worker task support for crowdsourcing/microtasks
- **Deadline Validation** - Automatic refunds after deadline passes
- **Rent Recovery** - Proper account closure returns SOL rent to client

## Deployed on Devnet

| Item | Value |
|------|-------|
| **Program ID** | `3iKABSF5zoQjGPykxUQNxbm7eQADqs8DreuGupggc679` |
| **Network** | Solana Devnet |

## Instructions

### Standard Escrow Flow

1. **create_escrow** - Client creates escrow with worker address and amount
2. **fund_escrow** - Client deposits USDC (worker_amount + platform_fee)
3. **release_escrow** - Client approves work, funds go to worker + treasury
4. **refund_escrow** - Client reclaims funds after deadline passes
5. **cancel_escrow** - Client cancels unfunded escrow

### Dispute Resolution

1. **freeze_escrow** - Client, worker, or admin freezes funded escrow
2. **admin_release_to_worker** - Admin releases to worker (platform keeps fee)
3. **admin_refund_to_client** - Admin refunds to client (platform keeps fee)
4. **admin_split_funds** - Admin splits funds between parties (platform keeps fee)

### Pool Escrow (Multi-Worker)

1. **create_pool_escrow** - Client creates pool with payment_per_worker and max_releases
2. **fund_pool_escrow** - Client deposits total budget
3. **partial_release** - Platform authority releases to individual workers
4. **close_pool_escrow** - Client closes pool and reclaims remaining funds

## Building

```bash
# Install Anchor CLI
cargo install --git https://github.com/coral-xyz/anchor avm --locked
avm install 0.32.1
avm use 0.32.1

# Build
anchor build

# Run tests
cargo test
```

## Security Features

- **Split validation** - Ensures splits sum to 100% and no duplicates
- **Worker pubkey validation** - Validates addresses are on ed25519 curve
- **Deadline limits** - Max 1 year deadline to prevent unrealistic escrows
- **Duplicate account checks** - Prevents same account used for multiple roles
- **Overflow protection** - Uses u128 intermediate calculations
- **Rent recovery** - `close = client` on account closures

## License

MIT License - see [LICENSE](LICENSE)

## About ZaphWork

ZaphWork is a decentralized freelance platform built on Solana blockchain. This escrow contract handles all payment flows between clients and workers.

- Website: [zaph.work](https://zaph.work)
