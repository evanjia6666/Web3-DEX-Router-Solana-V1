// devnet negative-case probes for uxuy-router (merged phase-2 build).
// Only the router-level guards are exercised — no DEX liquidity required.
const fs = require("fs");
const crypto = require("crypto");
const {
  Connection, Keypair, PublicKey, Transaction, TransactionInstruction,
  SystemProgram, sendAndConfirmTransaction,
} = require("@solana/web3.js");
const {
  TOKEN_PROGRAM_ID, MINT_SIZE, ACCOUNT_SIZE,
  createInitializeMint2Instruction, createInitializeAccount3Instruction,
} = require("@solana/spl-token");

const RPC = process.env.RPC_URL || "https://api.devnet.solana.com";
const PROGRAM_ID = new PublicKey("AN6zjJfTD5Cpj2x56sWsAod1Jb5viiQH4Xcg3tm5DgNY");
const AUTHORITY_PDA = new PublicKey("5rvVfdVCgSQ9AyTTctN1MPGk5brWVEtvMfPKtcesAuVG");
const ATA_PROGRAM = new PublicKey("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL");
const path = require("path");
const KEYS = process.env.PROBE_KEYS || path.join(__dirname, "keys.json");

const conn = new Connection(RPC, "confirmed");
const payer = Keypair.fromSecretKey(Uint8Array.from(
  JSON.parse(fs.readFileSync(process.env.PAYER_KEYPAIR || path.join(__dirname, "../../target/deploy/uxuy_devnet_payer-keypair.json")))
));

const disc = (name) => crypto.createHash("sha256").update("global:" + name).digest().subarray(0, 8);
const u16 = (n) => { const b = Buffer.alloc(2); b.writeUInt16LE(n); return b; };
const u32 = (n) => { const b = Buffer.alloc(4); b.writeUInt32LE(n); return b; };
const u64 = (n) => { const b = Buffer.alloc(8); b.writeBigUInt64LE(BigInt(n)); return b; };

// SwapArgs { order_id u64, amount_in u64, expect_amount_out u64, slippage u16,
//            routes: Vec<Vec<Route{dexes: Vec<Dex>, weights: Vec<u8>}>> }
function swapArgs({ amountIn = 1000n, expect = 0n, slippage = 0, route = [] } = {}) {
  return Buffer.concat([
    u64(0), u64(amountIn), u64(expect), u16(slippage),
    u32(1),                       // routes len
    u32(route.length ? 1 : 0),    // hops len
    ...(route.length ? [u32(route.length), Buffer.from(route), u32(100), Buffer.from([100])] : []),
  ]);
}

function ix(name, keys, data) {
  return new TransactionInstruction({
    programId: PROGRAM_ID,
    keys,
    data: Buffer.concat([disc(name), data]),
  });
}

async function ensureAccount(label, size, owner, initIx) {
  const store = fs.existsSync(KEYS) ? JSON.parse(fs.readFileSync(KEYS)) : {};
  const kp = store[label] ? Keypair.fromSecretKey(Uint8Array.from(store[label])) : Keypair.generate();
  store[label] = Array.from(kp.secretKey);
  fs.writeFileSync(KEYS, JSON.stringify(store));

  const info = await conn.getAccountInfo(kp.publicKey);
  if (!info) {
    const tx = new Transaction().add(
      SystemProgram.createAccount({
        fromPubkey: payer.publicKey, newAccountPubkey: kp.publicKey,
        lamports: await conn.getMinimumBalanceForRentExemption(size),
        space: size, programId: owner,
      }),
      initIx(kp.publicKey),
    );
    await sendAndConfirmTransaction(conn, tx, [payer, kp], { commitment: "confirmed" });
    console.log(`  created ${label}: ${kp.publicKey.toBase58()}`);
  }
  return kp.publicKey;
}

async function probe(label, tx, expectError) {
  tx.feePayer = payer.publicKey;
  tx.recentBlockhash = (await conn.getLatestBlockhash()).blockhash;
  tx.sign(payer);
  const sim = await conn.simulateTransaction(tx);
  const logs = (sim.value.logs || []).join("\n");
  const hit = logs.includes(expectError);
  console.log(`\n[${label}] expected ${expectError} -> ${hit ? "PASS" : "FAIL"}`);
  if (!hit) {
    console.log("  err:", JSON.stringify(sim.value.err));
    console.log("  logs tail:", logs.split("\n").slice(-12).join("\n    "));
  }
  return hit;
}

(async () => {
  const [pda, bump] = PublicKey.findProgramAddressSync([Buffer.from("uxuy_sa")], PROGRAM_ID);
  console.log("PDA derived:", pda.toBase58(), "bump", bump, "| matches constant:", pda.equals(AUTHORITY_PDA));

  const mint = await ensureAccount("mint", MINT_SIZE, TOKEN_PROGRAM_ID,
    (k) => createInitializeMint2Instruction(k, 6, payer.publicKey, null, TOKEN_PROGRAM_ID));
  const acctPayer = await ensureAccount("acct_payer", ACCOUNT_SIZE, TOKEN_PROGRAM_ID,
    (k) => createInitializeAccount3Instruction(k, mint, payer.publicKey, TOKEN_PROGRAM_ID));
  const acctPayer2 = await ensureAccount("acct_payer2", ACCOUNT_SIZE, TOKEN_PROGRAM_ID,
    (k) => createInitializeAccount3Instruction(k, mint, payer.publicKey, TOKEN_PROGRAM_ID));
  const acctPda = await ensureAccount("acct_pda", ACCOUNT_SIZE, TOKEN_PROGRAM_ID,
    (k) => createInitializeAccount3Instruction(k, mint, AUTHORITY_PDA, TOKEN_PROGRAM_ID));

  console.log("mint:", mint.toBase58());
  const results = [];

  // A) proxy: source == source_sa  -> SourceTokenSaIsSource
  results.push(await probe("A proxy same-account", new Transaction().add(ix("proxy_swap", [
    { pubkey: payer.publicKey, isSigner: true, isWritable: true },
    { pubkey: acctPda, isSigner: false, isWritable: true },        // source (PDA-owned)
    { pubkey: acctPayer2, isSigner: false, isWritable: true },     // destination
    { pubkey: mint, isSigner: false, isWritable: false },
    { pubkey: mint, isSigner: false, isWritable: false },
    { pubkey: AUTHORITY_PDA, isSigner: false, isWritable: false }, // sa_authority
    { pubkey: acctPda, isSigner: false, isWritable: true },        // source_token_sa == source
    { pubkey: PROGRAM_ID, isSigner: false, isWritable: false },    // destination_token_sa = None
    { pubkey: TOKEN_PROGRAM_ID, isSigner: false, isWritable: false },
    { pubkey: TOKEN_PROGRAM_ID, isSigner: false, isWritable: false },
    { pubkey: ATA_PROGRAM, isSigner: false, isWritable: false },
    { pubkey: SystemProgram.programId, isSigner: false, isWritable: false },
  ], swapArgs())), "SourceTokenSaIsSource"));

  // B) routes = [[]] -> EmptyRoute
  results.push(await probe("B empty route", new Transaction().add(ix("proxy_swap", [
    { pubkey: payer.publicKey, isSigner: true, isWritable: true },
    { pubkey: acctPayer, isSigner: false, isWritable: true },
    { pubkey: acctPayer2, isSigner: false, isWritable: true },
    { pubkey: mint, isSigner: false, isWritable: false },
    { pubkey: mint, isSigner: false, isWritable: false },
    { pubkey: PROGRAM_ID, isSigner: false, isWritable: false },     // sa_authority None
    { pubkey: PROGRAM_ID, isSigner: false, isWritable: false },     // source_token_sa None
    { pubkey: PROGRAM_ID, isSigner: false, isWritable: false },     // destination_token_sa None
    { pubkey: TOKEN_PROGRAM_ID, isSigner: false, isWritable: false },
    { pubkey: TOKEN_PROGRAM_ID, isSigner: false, isWritable: false },
    { pubkey: ATA_PROGRAM, isSigner: false, isWritable: false },
    { pubkey: SystemProgram.programId, isSigner: false, isWritable: false },
  ], swapArgs())), "EmptyRoute"));

  // C) swap_tob_v3 trim_rate=1 with no trailing account -> InvalidAccountsLength
  results.push(await probe("C trim short accounts", new Transaction().add(
    ix("swap_tob_v3", [
      { pubkey: payer.publicKey, isSigner: true, isWritable: true },
      { pubkey: acctPayer, isSigner: false, isWritable: true },
      { pubkey: acctPayer2, isSigner: false, isWritable: true },
      { pubkey: mint, isSigner: false, isWritable: false },
      { pubkey: mint, isSigner: false, isWritable: false },
      { pubkey: PROGRAM_ID, isSigner: false, isWritable: false },   // commission None
      { pubkey: PROGRAM_ID, isSigner: false, isWritable: false },   // platform fee None
      { pubkey: PROGRAM_ID, isSigner: false, isWritable: false },   // sa_authority None
      { pubkey: PROGRAM_ID, isSigner: false, isWritable: false },   // source_sa None
      { pubkey: PROGRAM_ID, isSigner: false, isWritable: false },   // dest_sa None
      { pubkey: TOKEN_PROGRAM_ID, isSigner: false, isWritable: false },
      { pubkey: TOKEN_PROGRAM_ID, isSigner: false, isWritable: false },
      { pubkey: ATA_PROGRAM, isSigner: false, isWritable: false },
      { pubkey: SystemProgram.programId, isSigner: false, isWritable: false },
    ], Buffer.concat([swapArgs(), u32(0), Buffer.from([1]), u16(0)]))
  ), "InvalidAccountsLength"));

  console.log(`\n=== ${results.filter(Boolean).length}/${results.length} passed ===`);
  process.exit(results.every(Boolean) ? 0 : 1);
})().catch((e) => { console.error("FATAL", e); process.exit(2); });
