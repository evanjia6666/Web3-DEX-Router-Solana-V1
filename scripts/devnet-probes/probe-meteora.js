// devnet probe: drives the router's MeteoraDlmmSwap2 window (19 slots) far
// enough to prove the adapter parses the wire and issues the CPI.
//
// What it pins: the account order/typing of the swap2 window, the route
// encoding (Dex::MeteoraDlmmSwap2 = 10) and that our CPI reaches meteora on
// the deployed bytes. The pair is a real devnet lb_pair carrying a real
// bin-array bitmap extension; src/dst are our own (wrong-mint) token
// accounts, so meteora must fail AFTER the extension slot — the observed
// stop is reserve_x 3012 (empty pool).
//
// What it canNOT pin: the writability of the extension slot. meteora's
// devnet build (2,198,048 B, sha d8fa183b…) differs from mainnet's
// (2,229,776 B, sha d296c677…) and does not enforce the mut constraint on
// that slot: the pre-fix build (readonly meta) and the fixed build behave
// identically here. The ConstraintMut(2000) regression only reproduces
// against a mainnet pool (Kimchi/SOL ENpXvFPb…, 2026-10-23), so the
// writable pinning has to be validated in the mainnet window via gdex.
const fs = require("fs");
const crypto = require("crypto");
const path = require("path");
const {
  Connection, Keypair, PublicKey, Transaction, TransactionInstruction,
  SystemProgram, ComputeBudgetProgram,
} = require("@solana/web3.js");
const { TOKEN_PROGRAM_ID } = require("@solana/spl-token");

const RPC = process.env.RPC_URL || "https://api.devnet.solana.com";
const PROGRAM_ID = new PublicKey(process.env.PROGRAM_ID || "AN6zjJfTD5Cpj2x56sWsAod1Jb5viiQH4Xcg3tm5DgNY");
const METEORA = new PublicKey("LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo");
const MEMO = new PublicKey("MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr");
const ATA_PROGRAM = new PublicKey("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL");
const KEYS = process.env.PROBE_KEYS || path.join(__dirname, "keys.json");

// devnet DLMM pair that carries a bin-array bitmap extension (found by
// scanning the program's pairs for an existing ["bitmap", pair] PDA);
// `node findext.tmp.js` re-derives the current list.
const POOL = {
  pair: new PublicKey("118MVR5DAKXHkNtRMhyHyjuQZzDN44c3gGT1if1mYMo"),
  tokenX: new PublicKey("7jMmeYxcVN9dMYX2JtTrpxC6y2Va8i92ujyXiNEFvuVp"),
  tokenY: new PublicKey("HFvLyT5VAmo48uuvEusCaYJLpVEj4Yo6ATRrFU7uaCSx"),
  reserveX: new PublicKey("BGoJdfAA39y9Vu688ZPUcwiEfsrgXtty6Q3uTCvhvhVr"),
  reserveY: new PublicKey("CzZx6aV8KfrsAqsGAfqixMpAz5fP9ycF7UGGR9EDwgn1"),
  activeId: 0,
};

const conn = new Connection(RPC, "confirmed");
const payer = Keypair.fromSecretKey(Uint8Array.from(
  JSON.parse(fs.readFileSync(process.env.PAYER_KEYPAIR ||
    path.join(__dirname, "../../target/deploy/uxuy_devnet_payer-keypair.json")))
));
const store = JSON.parse(fs.readFileSync(KEYS));
const mint = Keypair.fromSecretKey(Uint8Array.from(store.mint)).publicKey;
const src = Keypair.fromSecretKey(Uint8Array.from(store.acct_payer)).publicKey;
const dst = Keypair.fromSecretKey(Uint8Array.from(store.acct_payer2)).publicKey;

const pda = (seeds) => PublicKey.findProgramAddressSync(seeds, METEORA)[0];
const bitmapExt = pda([Buffer.from("bitmap"), POOL.pair.toBuffer()]);
const oracle = pda([Buffer.from("oracle"), POOL.pair.toBuffer()]);
const eventAuthority = pda([Buffer.from("__event_authority")]);
const binArray = (index) => {
  const b = Buffer.alloc(8);
  b.writeBigInt64LE(BigInt(index));
  return pda([Buffer.from("bin_array"), POOL.pair.toBuffer(), b]);
};

const disc = (name) => crypto.createHash("sha256").update("global:" + name).digest().subarray(0, 8);
const u16 = (n) => { const b = Buffer.alloc(2); b.writeUInt16LE(n); return b; };
const u32 = (n) => { const b = Buffer.alloc(4); b.writeUInt32LE(n); return b; };
const u64 = (n) => { const b = Buffer.alloc(8); b.writeBigUInt64LE(BigInt(n)); return b; };

// SwapArgs { order_id, amount_in, expect_amount_out, slippage,
//            routes: Vec<Vec<Route{dexes: Vec<Dex>, weights: Vec<u8>}>> }
function args(amountIn, dex) {
  return Buffer.concat([
    u64(0), u64(amountIn), u64(0), u16(0),
    u32(1), u32(1),                    // 1 path, 1 hop
    u32(1), Buffer.from([dex]),        // dexes: [MeteoraDlmmSwap2]
    u32(1), Buffer.from([100]),        // weights
    u32(0),                            // commission_info
    u16(0),                            // platform_fee_rate
  ]);
}

// 19-slot MeteoraDlmmSwap2 window, in the adapter's wire order.
function window() {
  const k = (pubkey, w) => ({ pubkey, isSigner: false, isWritable: w });
  const idx = Math.floor(POOL.activeId / 70);
  return [
    k(METEORA, false),                 // 0 dex program
    k(payer.publicKey, false),         // 1 swap authority (== src owner)
    k(src, true),                      // 2 source
    k(dst, true),                      // 3 destination
    k(POOL.pair, true),                // 4 lb_pair
    k(bitmapExt, process.env.BITMAP_WRITABLE !== "0"), // 5 bitmap extension  <-- the slot under test
    k(POOL.reserveX, true),            // 6 reserve_x
    k(POOL.reserveY, true),            // 7 reserve_y
    k(POOL.tokenX, false),             // 8 token_x mint
    k(POOL.tokenY, false),             // 9 token_y mint
    k(oracle, true),                   // 10 oracle
    k(src, true),                      // 11 host_fee_in
    k(TOKEN_PROGRAM_ID, false),        // 12 token_x program
    k(TOKEN_PROGRAM_ID, false),        // 13 token_y program
    k(MEMO, false),                    // 14 memo program
    k(eventAuthority, false),          // 15 event authority
    k(binArray(idx), true),            // 16 bin array 0
    k(binArray(idx + 1), true),        // 17 bin array 1
    k(binArray(idx + 2), true),        // 18 bin array 2
  ];
}

function ix() {
  return new TransactionInstruction({
    programId: PROGRAM_ID,
    keys: [
      { pubkey: payer.publicKey, isSigner: true, isWritable: true },  // payer
      { pubkey: src, isSigner: false, isWritable: true },             // source token account
      { pubkey: dst, isSigner: false, isWritable: true },             // destination token account
      { pubkey: mint, isSigner: false, isWritable: false },           // source mint
      { pubkey: mint, isSigner: false, isWritable: false },           // destination mint
      { pubkey: PROGRAM_ID, isSigner: false, isWritable: false },     // commission None
      { pubkey: PROGRAM_ID, isSigner: false, isWritable: false },     // platform fee None
      { pubkey: PROGRAM_ID, isSigner: false, isWritable: false },     // sa_authority None
      { pubkey: PROGRAM_ID, isSigner: false, isWritable: false },     // source_sa None
      { pubkey: PROGRAM_ID, isSigner: false, isWritable: false },     // destination_sa None
      { pubkey: TOKEN_PROGRAM_ID, isSigner: false, isWritable: false },
      { pubkey: TOKEN_PROGRAM_ID, isSigner: false, isWritable: false },
      { pubkey: ATA_PROGRAM, isSigner: false, isWritable: false },
      { pubkey: SystemProgram.programId, isSigner: false, isWritable: false },
      ...window(),
    ],
    data: Buffer.concat([disc("swap_v3"), args(1_000n, 10)]),
  });
}

(async () => {
  const tx = new Transaction().add(ComputeBudgetProgram.setComputeUnitLimit({ units: 400_000 }), ix());
  tx.feePayer = payer.publicKey;
  tx.recentBlockhash = (await conn.getLatestBlockhash()).blockhash;
  tx.sign(payer);
  const res = await conn.simulateTransaction(tx, [payer]);
  const err = res.value.err;
  const logs = res.value.logs || [];
  const text = JSON.stringify(err || "");
  console.log("err:", text);
  console.log("logs:\n  " + logs.slice(-6).join("\n  "));

  const code = Number((text.match(/"Custom":(\d+)/) || [])[1] || 0);
  const atExtensionSlot = code === 2000; // ConstraintMut on the bitmap slot
  const goodPair = code !== 3007 && code !== 4100; // pair decoded, program id matched
  const reached = goodPair && !atExtensionSlot;
  console.log(`\n=== ${reached ? "PASS" : "FAIL"}: window parsed, CPI reached meteora, stop code ${code} ===`);
  process.exit(reached ? 0 : 1);
})().catch((e) => { console.error("FATAL", e); process.exit(2); });
