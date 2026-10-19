use anchor_lang::prelude::*;

#[constant]
pub const SEED_SA: &[u8] = b"uxuy_sa";
pub const SEED_TEMP_WSOL: &[u8] = b"temp_wsol";
pub const BUMP_SA: u8 = 255;

pub const COMMISSION_RATE_LIMIT: u16 = 1_000; // 10%
pub const COMMISSION_DENOMINATOR: u64 = 10_000;

pub const COMMISSION_RATE_LIMIT_V2: u32 = 100_000_000; // 10%
pub const COMMISSION_DENOMINATOR_V2: u64 = 1_000_000_000;

pub const PLATFORM_FEE_RATE_LIMIT_V2: u64 = 1_000_000_000; // 100%
pub const PLATFORM_FEE_DENOMINATOR_V2: u64 = 1_000_000_000;

pub const TRIM_RATE_LIMIT_V2: u8 = 100; // 10%
pub const TRIM_DENOMINATOR_V2: u16 = 1_000;

pub const PLATFORM_FEE_RATE_LIMIT_V3: u64 = 10_000; // 100%
pub const PLATFORM_FEE_DENOMINATOR_V3: u64 = 10_000;

pub const MAX_HOPS: usize = 3;
pub const TOTAL_WEIGHT: u8 = 100;
pub const SA_AUTHORITY_SEED: &[&[&[u8]]] = &[&[SEED_SA, &[BUMP_SA]]];
pub const TOKEN_ACCOUNT_RENT: u64 = 2039280; // Token account rent (165 bytes)
pub const MIN_SOL_ACCOUNT_RENT: u64 = 890880;
pub const SOL_DIFF_LIMIT: u64 = 8_100_000;

// Actual amount_in lower bound ratio for post swap check
pub const ACTUAL_IN_LOWER_BOUND_NUM: u128 = 95; // 95%
pub const ACTUAL_IN_LOWER_BOUND_DEN: u128 = 100; // denominator for percentage

pub const SWAP_SELECTOR: &[u8; 8] = &[248, 198, 158, 145, 225, 117, 135, 200];
pub const SWAP2_SELECTOR: &[u8; 8] = &[65, 75, 63, 76, 235, 91, 91, 136];
pub const CPSWAP_SELECTOR: &[u8; 8] = &[143, 190, 90, 218, 196, 30, 51, 222];
pub const SWAP_V2_SELECTOR: &[u8; 8] = &[43, 4, 237, 11, 26, 201, 30, 98];
// byreal swap_v3_dyn (e52ed584692828e4): same args as swap_v2 (amount u64,
// other_amount_threshold u64, sqrt_price_limit_x64 u128, is_base_input u8);
// falls back to swap_v2 semantics on non-dynamic-fee pools, so it is safe
// to emit unconditionally — dynamic-fee pools reject plain swap_v2 with
// 6048 SwapDynamicFeeEnabled. Trailing: bitmap, ≤4 walk tick arrays, and
// the pyth oracle pair on dynamic-fee pools.
pub const SWAP_V3_DYN_SELECTOR: &[u8; 8] = &[229, 46, 213, 132, 105, 40, 40, 228];
pub const SWAP_EXACT_IN_SELECTOR: &[u8; 8] = &[104, 104, 131, 86, 161, 189, 180, 216];
pub const PLACE_TAKE_ORDER_SELECTOR: &[u8; 8] = &[3, 44, 71, 3, 26, 199, 203, 85];
pub const BRIDGE_TO_LOG_SELECTOR: &[u8; 8] = &[212, 189, 176, 218, 196, 135, 64, 122];
pub const ZERO_ADDRESS: Pubkey = Pubkey::new_from_array([0u8; 32]);
pub const HEAVEN_BUY_SELECTOR: &[u8; 8] = &[102, 6, 61, 18, 1, 218, 235, 234];
pub const HEAVEN_SELL_SELECTOR: &[u8; 8] = &[51, 230, 133, 164, 1, 127, 131, 173];

pub const PUMPFUN_BUY_SELECTOR: &[u8; 8] = &[102, 6, 61, 18, 1, 218, 235, 234];
pub const PUMPFUN_SELL_SELECTOR: &[u8; 8] = &[51, 230, 133, 164, 1, 127, 131, 173];
pub const MOONIT_BUY_SELECTOR: &[u8; 8] = &[102, 6, 61, 18, 1, 218, 235, 234];
pub const MOONIT_SELL_SELECTOR: &[u8; 8] = &[51, 230, 133, 164, 1, 127, 131, 173];
pub const STABBLE_SWAP_SELECTOR: &[u8; 8] = &[43, 4, 237, 11, 26, 201, 30, 98];
pub const DEPOSIT_SELECTOR: &[u8; 8] = &[242, 35, 198, 137, 82, 225, 242, 182];
pub const WITHDRAW_SELECTOR: &[u8; 8] = &[183, 18, 70, 156, 148, 109, 161, 34];
pub const REDEEM_SELECTOR: &[u8; 8] = &[184, 12, 86, 149, 70, 196, 97, 225];

pub const WOOFI_SWAP_SELECTOR: &[u8; 8] = &[248, 198, 158, 145, 225, 117, 135, 200];
pub const VIRTUALS_BUY_SELECTOR: &[u8; 8] = &[102, 6, 61, 18, 1, 218, 235, 234];
pub const VIRTUALS_SELL_SELECTOR: &[u8; 8] = &[51, 230, 133, 164, 1, 127, 131, 173];
pub const PERPETUALS_ADDLIQ_SELECTOR: &[u8; 8] = &[0xe4, 0xa2, 0x4e, 0x1c, 0x46, 0xdb, 0x74, 0x73];
pub const PERPETUALS_REMOVELIQ_SELECTOR: &[u8; 8] =
    &[0xe6, 0xd7, 0x52, 0x7f, 0xf1, 0x65, 0xe3, 0x92];
pub const PERPETUALS_SWAP_SELECTOR: &[u8; 8] = &[0x41, 0x4b, 0x3f, 0x4c, 0xeb, 0x5b, 0x5b, 0x88];
pub const BUY_EXACT_IN_SELECTOR: &[u8; 8] = &[250, 234, 13, 123, 213, 156, 19, 236];
pub const SELL_EXACT_IN_SELECTOR: &[u8; 8] = &[149, 39, 222, 155, 211, 124, 152, 26];
pub const VERTIGO_BUY_SELECTOR: &[u8; 8] = &[102, 6, 61, 18, 1, 218, 235, 234];
pub const VERTIGO_SELL_SELECTOR: &[u8; 8] = &[51, 230, 133, 164, 1, 127, 131, 173];
pub const BOOPFUN_BUY_SELECTOR: &[u8; 8] = &[138, 127, 14, 91, 38, 87, 115, 105];
pub const BOOPFUN_SELL_SELECTOR: &[u8; 8] = &[109, 61, 40, 187, 230, 176, 135, 174];
pub const GAMMA_ORACLE_SWAP_SELECTOR: &[u8; 8] = &[239, 82, 192, 187, 160, 26, 223, 223];
pub const SABER_DECIMAL_DEPOSIT_SELECTOR: &[u8; 8] =
    &[0xf2, 0x23, 0xc6, 0x89, 0x52, 0xe1, 0xf2, 0xb6];
pub const SABER_DECIMAL_WITHDRAW_SELECTOR: &[u8; 8] =
    &[0xb7, 0x12, 0x46, 0x9c, 0x94, 0x6d, 0xa1, 0x22];
pub const ONE_DEX_SWAP_SELECTOR: &[u8; 8] = &[8, 151, 245, 76, 172, 203, 144, 39];
pub const MANIFEST_SWAP_SELECTOR: &[u8; 1] = &[4];
pub const TESSERA_SWAP_SELECTOR: &[u8; 1] = &[16];
pub const SOL_RFQ_FILL_ORDER_SELECTOR: &[u8; 8] = &[232, 122, 115, 25, 199, 143, 136, 162];
pub const SPOT_SWAP_SELECTOR: &[u8; 8] = &[167, 97, 12, 231, 237, 78, 166, 251];

pub const HUMIDIFI_SWAP_SELECTOR: u8 = 0x4;
pub const HUMIDIFI_SWAP_WITH_USER_DATA_SELECTOR: u8 = 0x14;
const HUMIDIFI_IX_DATA_KEY_SEED: [u8; 32] = [
    58, 255, 47, 255, 226, 186, 235, 195, 123, 131, 245, 8, 11, 233, 132, 219, 225, 40, 79, 119,
    169, 121, 169, 58, 197, 1, 122, 9, 216, 164, 149, 97,
];
pub const HUMIDIFI_IX_DATA_KEY: u64 = u64::from_le_bytes([
    HUMIDIFI_IX_DATA_KEY_SEED[0],
    HUMIDIFI_IX_DATA_KEY_SEED[1],
    HUMIDIFI_IX_DATA_KEY_SEED[2],
    HUMIDIFI_IX_DATA_KEY_SEED[3],
    HUMIDIFI_IX_DATA_KEY_SEED[4],
    HUMIDIFI_IX_DATA_KEY_SEED[5],
    HUMIDIFI_IX_DATA_KEY_SEED[6],
    HUMIDIFI_IX_DATA_KEY_SEED[7],
]);
pub const ALPHAQ_SWAP_SELECTOR: &[u8; 1] = &[0x0c];
pub const TAURUSFI_SWAP_SELECTOR: &[u8; 1] = &[5];

pub const SWAP_SELECTOR_02: &[u8; 1] = &[2]; // 02
pub const GOONFI_SWAP_SELECTOR: &[u8; 1] = SWAP_SELECTOR_02;
pub const SCORCH_SWAP_SELECTOR: &[u8; 1] = SWAP_SELECTOR_02;
pub const BISONFI_SWAP_SELECTOR: &[u8; 1] = SWAP_SELECTOR_02;

pub const BUY_EXACT_SOL_IN_SELECTOR: &[u8; 8] = &[56, 252, 116, 8, 158, 223, 205, 95]; // 38 fc 74 08 9e df cd 5f
pub const BUY_EXACT_QUOTE_IN_SELECTOR: &[u8; 8] = &[198, 46, 21, 82, 180, 217, 232, 112]; // c6 2e 15 52 b4 d9 e8 70
pub const GOONFI_V2_SWAP_SELECTOR: &[u8; 1] = &[1];
pub const QUANTUM_SWAP_SELECTOR: &[u8; 1] = &[7];

// authority_pda: PDA(["uxuy_sa"], program_id), bump 255 — signs intermediate hops & fee transfers
pub mod authority_pda {
    use anchor_lang::prelude::*;
    pub const ID: Pubkey = Pubkey::from_str_const("5rvVfdVCgSQ9AyTTctN1MPGk5brWVEtvMfPKtcesAuVG");
    pub fn id() -> Pubkey { ID }
}

// wsol_sa: authority_pda ATA for WSOL
pub mod wsol_sa {
    use anchor_lang::prelude::*;
    pub const ID: Pubkey = Pubkey::from_str_const("G6KLoMzx47Nh1xJLtzyGiFXiwH688Z8LEwHtFv4xBPyj");
    pub fn id() -> Pubkey { ID }
}

// claim_authority: uxuy ops wallet allowed to call `claim`
pub mod claim_authority {
    use anchor_lang::prelude::*;
    pub const ID: Pubkey = Pubkey::from_str_const("J8NpT3tG57ggTbr4ek8qwapG9SeGnTfXmNZn6ncjuRsG");
    pub fn id() -> Pubkey { ID }
}

// token_program
pub mod token_program {
    use anchor_lang::prelude::*;
    pub const ID: Pubkey = Pubkey::from_str_const("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA");
    pub fn id() -> Pubkey { ID }
}

// token_2022_program
pub mod token_2022_program {
    use anchor_lang::prelude::*;
    pub const ID: Pubkey = Pubkey::from_str_const("TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb");
    pub fn id() -> Pubkey { ID }
}

// wsol_program: WSOL mint
pub mod wsol_program {
    use anchor_lang::prelude::*;
    pub const ID: Pubkey = Pubkey::from_str_const("So11111111111111111111111111111111111111112");
    pub fn id() -> Pubkey { ID }
}

// system_program: backend uses this address to represent native sol
pub mod system_program {
    use anchor_lang::prelude::*;
    pub const ID: Pubkey = Pubkey::from_str_const("11111111111111111111111111111111");
    pub fn id() -> Pubkey { ID }
}

// raydium_swap_program: Raydium AMM v4
pub mod raydium_swap_program {
    use anchor_lang::prelude::*;
    pub const ID: Pubkey = Pubkey::from_str_const("675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8");
    pub fn id() -> Pubkey { ID }
}

// raydium_stable_program
pub mod raydium_stable_program {
    use anchor_lang::prelude::*;
    pub const ID: Pubkey = Pubkey::from_str_const("5quBtoiQqxF9Jv6KYKctB59NT3gtJD2Y65kdnB1Uev3h");
    pub fn id() -> Pubkey { ID }
}

// raydium_clmm_program
pub mod raydium_clmm_program {
    use anchor_lang::prelude::*;
    pub const ID: Pubkey = Pubkey::from_str_const("CAMMCzo5YL8w4VFF8KVHrK22GGUsp5VTaW7grrKgrWqK");
    pub fn id() -> Pubkey { ID }
}

// raydium_cpmm_program
pub mod raydium_cpmm_program {
    use anchor_lang::prelude::*;
    pub const ID: Pubkey = Pubkey::from_str_const("CPMMoo8L3F4NbTegBCKVNunggL7H1ZpdTHKxQB5qKP1C");
    pub fn id() -> Pubkey { ID }
}

// whirlpool_program: Orca Whirlpool
pub mod whirlpool_program {
    use anchor_lang::prelude::*;
    pub const ID: Pubkey = Pubkey::from_str_const("whirLbMiicVdio4qvUfM5KAg6Ct8VwpYzGff3uctyCc");
    pub fn id() -> Pubkey { ID }
}

// pancake_swap_v3_program: PancakeSwap CLMM
pub mod pancake_swap_v3_program {
    use anchor_lang::prelude::*;
    pub const ID: Pubkey = Pubkey::from_str_const("HpNfyc2Saw7RKkQd8nEL4khUcuPhQ7WwY1B2qjx8jxFq");
    pub fn id() -> Pubkey { ID }
}

// meteora_dlmm_program: Meteora DLMM
pub mod meteora_dlmm_program {
    use anchor_lang::prelude::*;
    pub const ID: Pubkey = Pubkey::from_str_const("LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo");
    pub fn id() -> Pubkey { ID }
}

// meteora_dynamicpool_program
pub mod meteora_dynamicpool_program {
    use anchor_lang::prelude::*;
    pub const ID: Pubkey = Pubkey::from_str_const("Eo7WjKq67rjJQSZxS6z3YkapzY3eMj6Xy8X5EQVn5UaB");
    pub fn id() -> Pubkey { ID }
}

// meteora_vault_program
pub mod meteora_vault_program {
    use anchor_lang::prelude::*;
    pub const ID: Pubkey = Pubkey::from_str_const("24Uqj9JCLxUeoC3hGfh5W3s9FM9uCHDS2SG3LYwBpyTi");
    pub fn id() -> Pubkey { ID }
}

// meteora_damm_v2_program
pub mod meteora_damm_v2_program {
    use anchor_lang::prelude::*;
    pub const ID: Pubkey = Pubkey::from_str_const("cpamdpZCGKUy5JxQXB4dcpGPiikHawvSWAd6mEn1sGG");
    pub fn id() -> Pubkey { ID }
}

// pumpfunamm_program: PumpSwap
pub mod pumpfunamm_program {
    use anchor_lang::prelude::*;
    pub const ID: Pubkey = Pubkey::from_str_const("pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA");
    pub fn id() -> Pubkey { ID }
}

// byreal_clmm_program: Byreal CLMM
pub mod byreal_clmm_program {
    use anchor_lang::prelude::*;
    pub const ID: Pubkey = Pubkey::from_str_const("REALQqNEomY6cQGZJUGwywTBD2UmDT32rZcNnfxQ5N2");
    pub fn id() -> Pubkey { ID }
}

// pumpfun_program: pump.fun bonding curve (pre-graduation trading)
pub mod pumpfun_program {
    use anchor_lang::prelude::*;
    pub const ID: Pubkey = Pubkey::from_str_const("6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P");
    pub fn id() -> Pubkey { ID }
}

// Headroom the pump.fun curve buy keeps available on the swap authority on
// top of the spend itself: pump may idempotently create the curve's creator
// vault (rent.minimum_balance(0) = 890_880) and the user volume accumulator
// (rent 1_346_280 observed on chain) from the `user` account. Fees ride
// inside spendable_sol_in (net-of-fee formula), so rents are the only
// extra native cost.
pub const PUMPFUN_FUND_HEADROOM: u64 = 2_400_000;
