// mainnet, simulate-only: buy Kimchi (tokenX) with WSOL against Meteora DLMM
// pool ENpXvFPb… — the pool whose swap died live with ConstraintMut(2000)
// while the router forwarded the bin-array bitmap extension read-only.
//
// Nothing is signed for fee payment or sent: the transaction is simulated
// against mainnet state (WSOL ATA + wrap are synthesized inside the same
// simulated tx), so it costs nothing and changes nothing.
//
// Expect: err=null, log "Dex::MeteoraDlmmSwap2" → meteora "Instruction: Swap2"
// → "SwapEvent { dex: MeteoraDlmmSwap2, ... }".
//
// Requires the ops keypair only as the simulated user (it pays nothing).
const fs = require("fs");
const crypto = require("crypto");
const path = require("path");
const {
  Connection, Keypair, PublicKey, Transaction, TransactionInstruction,
  SystemProgram, ComputeBudgetProgram,
} = require("@solana/web3.js");
const {
  TOKEN_PROGRAM_ID, createAssociatedTokenAccountIdempotentInstruction,
  createSyncNativeInstruction, getAssociatedTokenAddressSync,
} = require("@solana/spl-token");

const RPC = "https://solana-mainnet.g.alchemy.com/v2/xYffEY4JKceyYd2UjR-r9";
const PROGRAM_ID = new PublicKey("AN6zjJfTD5Cpj2x56sWsAod1Jb5viiQH4Xcg3tm5DgNY");
const METEORA = new PublicKey("LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo");
const MEMO = new PublicKey("MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr");
const ATA_PROGRAM = new PublicKey("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL");
const WSOL = new PublicKey("So11111111111111111111111111111111111111112");
const TOKEN_2022 = new PublicKey("TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb");

const POOL = {
  pair: new PublicKey("ENpXvFPbJkShkk7UqqXqpEYA9LHpj9Yt4wEofnGNdA9R"),
  tokenX: new PublicKey("CSocnzcRpgSCr1naKcNTieWZUxb6rABe45j844m7pump"),
  tokenY: WSOL,
  reserveX: new PublicKey("A46KyRPi8rca9dhg5ctXy3VoYF3KVijFCtnr982tPYta"),
  reserveY: new PublicKey("7oQZ2BS4PrWV8zNyBAmNvxmmtk9eDgB7yRucRoafRoGT"),
  oracle: new PublicKey("4ygnnyfLkNPu5S7ghg37dLbZXj5knfTqZbA7dhCE3THs"),
  bitmap: new PublicKey("EhsDKsEvDCJ2WQibxbAyz2c5CRUYKvBMHjMMiDLLS9WZ"),
  bins: [
    new PublicKey("9kp5VvbFq7YtDiKzZzS9mznhfPmkbVzgqYjfwH3DULNs"),
    new PublicKey("H2gUh8guXohJqucXRJSkKYkPWFgtVc7udfcdUfDEFaQn"),
    new PublicKey("HxDbjsi15wv9WSunDpQpgVKqM4nevxCzUV2oyoYZ8ohY"),
  ],
};

const conn = new Connection(RPC, "confirmed");
const payer = Keypair.fromSecretKey(Uint8Array.from(
  JSON.parse(fs.readFileSync(path.join(__dirname, "../../target/deploy/uxuy_ops_payer-keypair.json")))
));

const disc = (n) => crypto.createHash("sha256").update("global:" + n).digest().subarray(0, 8);
const u16 = (n) => { const b = Buffer.alloc(2); b.writeUInt16LE(n); return b; };
const u32 = (n) => { const b = Buffer.alloc(4); b.writeUInt32LE(n); return b; };
const u64 = (n) => { const b = Buffer.alloc(8); b.writeBigUInt64LE(BigInt(n)); return b; };

const AMOUNT = 20_000_000n; // 0.02 SOL
function swapArgs(dex) {
  return Buffer.concat([
    u64(0), u64(AMOUNT), u64(0), u16(0),
    u32(1), u32(1),
    u32(1), Buffer.from([dex]),
    u32(1), Buffer.from([100]),
    u32(0), u16(0),
  ]);
}

const k = (pubkey, w) => ({ pubkey, isSigner: false, isWritable: w });
function window(src, dst) {
  return [
    k(METEORA, false),
    k(payer.publicKey, false),
    k(src, true),
    k(dst, true),
    k(POOL.pair, true),
    k(POOL.bitmap, true),
    k(POOL.reserveX, true),
    k(POOL.reserveY, true),
    k(POOL.tokenX, false),
    k(POOL.tokenY, false),
    k(POOL.oracle, true),
    k(src, true),          // host_fee_in = input side
    k(TOKEN_2022, false),   // token_x (Kimchi is Token-2022)
    k(TOKEN_PROGRAM_ID, false),
    k(MEMO, false),
    k(new PublicKey("D1ZN9Wj1fRSUQfCjhvnu1hqDMT7hzjzBBpi12nVniYD6"), false),
    k(POOL.bins[0], true),
    k(POOL.bins[1], true),
    k(POOL.bins[2], true),
  ];
}

(async () => {
  const wsolAta = getAssociatedTokenAddressSync(WSOL, payer.publicKey);
  const dstAta = getAssociatedTokenAddressSync(POOL.tokenX, payer.publicKey, false, TOKEN_2022);
  const swapIx = new TransactionInstruction({
    programId: PROGRAM_ID,
    keys: [
      { pubkey: payer.publicKey, isSigner: true, isWritable: true },
      { pubkey: wsolAta, isSigner: false, isWritable: true },
      { pubkey: dstAta, isSigner: false, isWritable: true },
      { pubkey: WSOL, isSigner: false, isWritable: false },
      { pubkey: POOL.tokenX, isSigner: false, isWritable: false },
      { pubkey: PROGRAM_ID, isSigner: false, isWritable: false },
      { pubkey: PROGRAM_ID, isSigner: false, isWritable: false },
      { pubkey: PROGRAM_ID, isSigner: false, isWritable: false },
      { pubkey: PROGRAM_ID, isSigner: false, isWritable: false },
      { pubkey: PROGRAM_ID, isSigner: false, isWritable: false },
      { pubkey: TOKEN_PROGRAM_ID, isSigner: false, isWritable: false },
      { pubkey: TOKEN_2022, isSigner: false, isWritable: false },
      { pubkey: ATA_PROGRAM, isSigner: false, isWritable: false },
      { pubkey: SystemProgram.programId, isSigner: false, isWritable: false },
      ...window(wsolAta, dstAta),
    ],
    data: Buffer.concat([disc("swap_v3"), swapArgs(10)]), // Dex::MeteoraDlmmSwap2
  });

  const tx = new Transaction().add(
    createAssociatedTokenAccountIdempotentInstruction(payer.publicKey, wsolAta, payer.publicKey, WSOL),
    SystemProgram.transfer({ fromPubkey: payer.publicKey, toPubkey: wsolAta, lamports: Number(AMOUNT) }),
    createSyncNativeInstruction(wsolAta),
    createAssociatedTokenAccountIdempotentInstruction(payer.publicKey, dstAta, payer.publicKey, POOL.tokenX, TOKEN_2022),
    ComputeBudgetProgram.setComputeUnitLimit({ units: 400_000 }),
    swapIx,
  );
  tx.feePayer = payer.publicKey;
  tx.recentBlockhash = (await conn.getLatestBlockhash()).blockhash;
  tx.sign(payer);
  const r = await conn.simulateTransaction(tx, [payer]);
  console.log("err:", JSON.stringify(r.value.err));
  console.log("CU:", r.value.unitsConsumed);
  console.log((r.value.logs || []).filter(l => /Dex::|Instruction:|Error|error|Swap|swap/.test(l)).slice(-12).join("\n"));
})();
