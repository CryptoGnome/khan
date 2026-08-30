Operates EVM-chain wallets (Optimism, Base, Ethereum, Arbitrum) safely from a Solana-native company — gas, nonces, chain IDs, checksums, and the mistakes that send funds to the wrong chain. Load before any transaction, balance check, or wallet setup on an EVM chain, including Farcaster registration and anything on OP-Mainnet.

# EVM wallets for a Solana-native company

Everything this company knows about wallets is Solana-shaped: rent, ATAs,
priority fees, base58. EVM chains share none of it. These are the rules that
prevent the expensive first-timer mistakes.

## When to use
- Creating or funding an EVM wallet; sending any EVM transaction; verifying
  an EVM balance. Farcaster (OP-Mainnet) work always starts here.
- Not for Solana operations — existing skills cover those.

## Quick reference
- One keypair = the SAME address on every EVM chain. Which chain a transfer
  lands on is decided ONLY by the chain ID it was signed for / sent to.
  Funds sent "to the right address on the wrong chain" sit on that other
  chain — recoverable only by signing there, lost only if the key isn't ours.
- Gas is paid in the chain's NATIVE coin: ETH on Optimism/Base/Arbitrum/
  mainnet. A wallet holding only USDC on OP cannot move at all — always keep
  a gas reserve (~$2-3 of ETH) beside any tokens.
- Addresses are hex with an EIP-55 checksum (mixed case). Compare
  case-insensitively, but SUBMIT the checksummed form; a checksum mismatch
  from a doc/source is a red flag, not a formatting quirk.
- Public RPCs (no key): Optimism `https://mainnet.optimism.io`, Base
  `https://mainnet.base.org`. Balance: `eth_getBalance`; nonce:
  `eth_getTransactionCount`; receipt: `eth_getTransactionReceipt`
  (`status: 0x1` = success).
- Fees are EIP-1559: maxFeePerGas + maxPriorityFeePerGas; the node's
  `eth_feeHistory`/`eth_estimateGas` size them. L2 fees are cents — a quote
  in dollars means something is wrong.

## Procedure
1. Wallet: generate the keypair ourselves, store and back up the key in the
   vault with treasury discipline. Record the address AND the intended chain
   next to it — an address without a chain label caused wrong-chain sends.
2. Fund with the native coin first (bridge ETH per bridge_hygiene), verify
   with eth_getBalance on the target chain's own RPC before anything else.
3. Every transaction: correct chain ID, estimated gas, then sign LOCALLY —
   the key never leaves our process, never goes in argv or logs.
4. After sending: poll eth_getTransactionReceipt until status 0x1. A tx hash
   without a success receipt is not done. status 0x0 = reverted — the gas is
   spent, the action did not happen; read the revert before retrying.
5. Book it: chain, tx hash, gas paid, purpose.

## Pitfalls
- Nonces are strictly sequential per address per chain: a stuck low-fee tx
  blocks every later one — replace it (same nonce, higher fee), don't queue
  more behind it.
- eth_getBalance returns wei (10^18 per ETH) — decimal mistakes here are
  1000x mistakes.
- ERC-20 balances need the token contract's balanceOf, not eth_getBalance;
  token transfers are contract calls and cost more gas than plain sends.
- Never grant unlimited ERC-20 allowances; approve the exact amount needed.
- "Same address works everywhere" cuts both ways: double-check the CHAIN
  every time an address is pasted anywhere.

## Verification
The action's effect is visible on the target chain's own RPC (balance moved,
receipt status 0x1, contract state changed), and the ledger entry names the
chain. If the effect is only visible on a bridge page, an explorer of a
DIFFERENT chain, or in our own notes, it is not verified.
