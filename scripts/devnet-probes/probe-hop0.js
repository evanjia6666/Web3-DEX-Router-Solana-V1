// devnet probes #2: create_pump_hop0_temp (positive) + hop0 pump guards.
const fs = require("fs");
const crypto = require("crypto");
const path = require("path");
const {
  Connection, Keypair, PublicKey, Transaction, TransactionInstruction,
  SystemProgram, sendAndConfirmTransaction,
} = require("@solana/web3.js");
const { TOKEN_PROGRAM_ID } = require("@solana/spl-token");

const RPC = process.env.RPC_URL || "https://api.devnet.solana.com";
const PROGRAM_ID = new PublicKey("AN6zjJfTD5Cpj2x56sWsAod1Jb5viiQH4Xcg3tm5DgNY");
const AUTHORITY_PDA = new PublicKey("5rvVfdVCgSQ9AyTTctN1MPGk5brWVEtvMfPKtcesAuVG");
const PUMP = new PublicKey("6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P");
const ATA_PROGRAM = new PublicKey("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL");

const conn = new Connection(RPC, "confirmed");
const payer = Keypair.fromSecretKey(Uint8Array.from(JSON.parse(fs.readFileSync(process.env.PAYER_KEYPAIR || path.join(__dirname, "../../target/deploy/uxuy_devnet_payer-keypair.json")))));
const KEYS = process.env.PROBE_KEYS || path.join(__dirname, "keys.json");
const store = JSON.parse(fs.readFileSync(KEYS));
const mint = Keypair.fromSecretKey(Uint8Array.from(store.mint)).publicKey;
const acctPayer2 = Keypair.fromSecretKey(Uint8Array.from(store.acct_payer2)).publicKey;
const acctPda = Keypair.fromSecretKey(Uint8Array.from(store.acct_pda)).publicKey;

const disc = (n) => crypto.createHash("sha256").update("global:" + n).digest().subarray(0, 8);
const u16 = (n) => { const b = Buffer.alloc(2); b.writeUInt16LE(n); return b; };
const u32 = (n) => { const b = Buffer.alloc(4); b.writeUInt32LE(n); return b; };
const u64 = (n) => { const b = Buffer.alloc(8); b.writeBigUInt64LE(BigInt(n)); return b; };

function swapArgs({ amountIn = 10_000_000n, dex = null } = {}) {
  const hops = dex === null ? [] : [u32(1), Buffer.from([dex]), u32(1), Buffer.from([100])];
  return Buffer.concat([u64(0), u64(amountIn), u64(0n), u16(0), u32(1), u32(dex === null ? 0 : 1), ...hops]);
}

async function probe(label, tx, expect) {
  tx.feePayer = payer.publicKey;
  tx.recentBlockhash = (await conn.getLatestBlockhash()).blockhash;
  tx.sign(payer);
  const sim = await conn.simulateTransaction(tx);
  const logs = (sim.value.logs || []).join("\n");
  const hit = logs.includes(expect);
  console.log(`[${label}] ${hit ? "PASS" : "FAIL"} (want ${expect})`);
  if (!hit) console.log("  err:", JSON.stringify(sim.value.err), "\n  ", logs.split("\n").slice(-8).join("\n   "));
  return hit;
}

(async () => {
  const [temp, bump] = PublicKey.findProgramAddressSync([Buffer.from("pump-hop0"), payer.publicKey.toBuffer()], PROGRAM_ID);
  console.log("hop0 temp PDA:", temp.toBase58(), "bump", bump);

  // 1) create_pump_hop0_temp — real tx (the instruction was dead code before the fix)
  if (!(await conn.getAccountInfo(temp))) {
    const tx = new Transaction().add(new TransactionInstruction({
      programId: PROGRAM_ID,
      keys: [
        { pubkey: payer.publicKey, isSigner: true, isWritable: true },
        { pubkey: temp, isSigner: false, isWritable: true },
        { pubkey: AUTHORITY_PDA, isSigner: false, isWritable: false },
        { pubkey: mint, isSigner: false, isWritable: false },
        { pubkey: TOKEN_PROGRAM_ID, isSigner: false, isWritable: false },
        { pubkey: SystemProgram.programId, isSigner: false, isWritable: false },
      ],
      data: Buffer.concat([disc("create_pump_hop0_temp"), Buffer.from([bump])]),
    }));
    const sig = await sendAndConfirmTransaction(conn, tx, [payer], { commitment: "confirmed" });
    console.log("  create tx:", sig);
  } else {
    console.log("  temp already exists, inspecting");
  }

  const info = await conn.getAccountInfo(temp);
  const d = info.data;
  const tokenOwner = new PublicKey(d.subarray(32, 64)).toBase58();
  console.log("  temp: owner program", info.owner.toBase58(), "len", d.length,
    "| mint", new PublicKey(d.subarray(0, 32)).toBase58(),
    "| token owner", tokenOwner, "| amount", d.readBigUInt64LE(64).toString(),
    "| state", d[108]);
  const createOk = info.owner.equals(TOKEN_PROGRAM_ID) && d.length === 165 &&
    d[108] === 1 && tokenOwner === AUTHORITY_PDA.toBase58() &&
    new PublicKey(d.subarray(0, 32)).equals(mint);
  console.log(`[create_pump_hop0_temp initialized] ${createOk ? "PASS" : "FAIL"}`);

  // shared 20-slot pump curve buy window
  const window = (source) => [
    { pubkey: PUMP, isSigner: false, isWritable: false },              // dex program
    { pubkey: AUTHORITY_PDA, isSigner: false, isWritable: false },     // swap authority (user)
    { pubkey: source, isSigner: false, isWritable: true },             // source
    { pubkey: acctPayer2, isSigner: false, isWritable: true },         // destination
    { pubkey: payer.publicKey, isSigner: false, isWritable: false },   // global
    { pubkey: payer.publicKey, isSigner: false, isWritable: false },   // fee recipient
    { pubkey: mint, isSigner: false, isWritable: false },              // mint
    { pubkey: payer.publicKey, isSigner: false, isWritable: false },   // bonding curve
    { pubkey: payer.publicKey, isSigner: false, isWritable: false },   // assoc bonding curve
    { pubkey: SystemProgram.programId, isSigner: false, isWritable: false },
    { pubkey: TOKEN_PROGRAM_ID, isSigner: false, isWritable: false },  // src token program
    { pubkey: TOKEN_PROGRAM_ID, isSigner: false, isWritable: false },  // base token program
    { pubkey: payer.publicKey, isSigner: false, isWritable: false },   // creator vault
    { pubkey: payer.publicKey, isSigner: false, isWritable: false },   // event authority
    { pubkey: payer.publicKey, isSigner: false, isWritable: false },   // gva
    { pubkey: payer.publicKey, isSigner: false, isWritable: false },   // uva
    { pubkey: payer.publicKey, isSigner: false, isWritable: false },   // fee config
    { pubkey: payer.publicKey, isSigner: false, isWritable: false },   // fee program
    { pubkey: payer.publicKey, isSigner: false, isWritable: false },   // buyback a
    { pubkey: payer.publicKey, isSigner: false, isWritable: false },   // buyback b
  ];

  const proxyIx = (source) => new TransactionInstruction({
    programId: PROGRAM_ID,
    keys: [
      { pubkey: payer.publicKey, isSigner: true, isWritable: true },
      { pubkey: source, isSigner: false, isWritable: true },
      { pubkey: acctPayer2, isSigner: false, isWritable: true },
      { pubkey: mint, isSigner: false, isWritable: false },
      { pubkey: mint, isSigner: false, isWritable: false },
      { pubkey: PROGRAM_ID, isSigner: false, isWritable: false },   // sa_authority None
      { pubkey: PROGRAM_ID, isSigner: false, isWritable: false },   // source_sa None
      { pubkey: PROGRAM_ID, isSigner: false, isWritable: false },   // dest_sa None
      { pubkey: TOKEN_PROGRAM_ID, isSigner: false, isWritable: false },
      { pubkey: TOKEN_PROGRAM_ID, isSigner: false, isWritable: false },
      { pubkey: ATA_PROGRAM, isSigner: false, isWritable: false },
      { pubkey: SystemProgram.programId, isSigner: false, isWritable: false },
      ...window(source),
    ],
    data: Buffer.concat([disc("proxy_swap"), swapArgs({ amountIn: 10_000_000n, dex: 15 })]),
  });

  const r = [];
  // D) unfunded hop0 temp: amount_in (0.01 SOL) > temp lamports (rent only)
  r.push(await probe("D unfunded hop0 temp", new Transaction().add(proxyIx(temp)), "InvalidSourceTokenAccount"));
  // E) custody (PDA-owned, not the hop0 temp) as hop0 source
  r.push(await probe("E custody as hop0 source", new Transaction().add(proxyIx(acctPda)), "InvalidHopAccounts"));

  console.log(`\n=== create:${createOk ? 1 : 0}/1, guards:${r.filter(Boolean).length}/2 ===`);
  process.exit(createOk && r.every(Boolean) ? 0 : 1);
})().catch((e) => { console.error("FATAL", e); process.exit(2); });
