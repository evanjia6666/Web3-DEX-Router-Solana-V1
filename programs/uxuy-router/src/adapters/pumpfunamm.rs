use super::common::DexProcessor;
use crate::adapters::common::{
    before_check, before_check_pda_user, enforce_sa_token_allowlist, invoke_process,
};
use crate::error::ErrorCode;
use crate::utils::{close_token_account, transfer_sol};
use crate::{
    BUY_EXACT_QUOTE_IN_SELECTOR, HopAccounts, PUMPFUN_SELL_SELECTOR, PUMPFUNAMM_FUND_HEADROOM,
    SA_AUTHORITY_SEED, SOL_DIFF_LIMIT, authority_pda, pumpfunamm_program, wsol_sa,
};
use anchor_lang::prelude::*;
use anchor_lang::solana_program::instruction::Instruction;
use anchor_spl::token_interface::TokenAccount;
use arrayref::array_ref;
const ARGS_LEN: usize = 25;
pub struct PumpfunammSellAccounts3<'info> {
    pub dex_program_id: &'info AccountInfo<'info>,
    pub swap_authority_pubkey: &'info AccountInfo<'info>,
    pub swap_source_token: InterfaceAccount<'info, TokenAccount>,
    pub swap_destination_token: InterfaceAccount<'info, TokenAccount>,

    pub pool: &'info AccountInfo<'info>,
    pub global_config: &'info AccountInfo<'info>,
    pub base_mint: &'info AccountInfo<'info>,
    pub quote_mint: &'info AccountInfo<'info>,
    pub pool_base_token_account: &'info AccountInfo<'info>,
    pub pool_quote_token_account: &'info AccountInfo<'info>,
    pub protocol_fee_recipient: &'info AccountInfo<'info>,
    pub protocol_fee_recipient_token_account: &'info AccountInfo<'info>,
    pub base_token_program: &'info AccountInfo<'info>,
    pub quote_token_program: &'info AccountInfo<'info>,
    pub system_program: &'info AccountInfo<'info>,
    pub associated_token_program: &'info AccountInfo<'info>,
    pub event_authority: &'info AccountInfo<'info>,
    pub coin_creator_vault_ata: &'info AccountInfo<'info>,
    pub coin_creator_vault_authority: &'info AccountInfo<'info>,
    pub fee_config: &'info AccountInfo<'info>,
    pub fee_program: &'info AccountInfo<'info>,
    // 2026-09 pump_amm revision: buy/sell REQUIRE trailing accounts —
    // pool-v2 (only when the pool's coin_creator is set; unused slots
    // carry the dex program id and are skipped), then the buyback fee
    // recipient (readonly) and its quote-mint ATA (writable)
    pub pool_v2: &'info AccountInfo<'info>,
    pub buyback_fee_recipient: &'info AccountInfo<'info>,
    pub buyback_fee_recipient_quote_ata: &'info AccountInfo<'info>,
}
const SELL_ACCOUNTS_LEN3: usize = 24;

impl<'info> PumpfunammSellAccounts3<'info> {
    fn parse_accounts(accounts: &'info [AccountInfo<'info>], offset: usize) -> Result<Self> {
        let [
            dex_program_id,
            swap_authority_pubkey,
            swap_source_token,
            swap_destination_token,
            pool,
            global_config,
            base_mint,
            quote_mint,
            pool_base_token_account,
            pool_quote_token_account,
            protocol_fee_recipient,
            protocol_fee_recipient_token_account,
            base_token_program,
            quote_token_program,
            system_program,
            associated_token_program,
            event_authority,
            coin_creator_vault_ata,
            coin_creator_vault_authority,
            fee_config,
            fee_program,
            pool_v2,
            buyback_fee_recipient,
            buyback_fee_recipient_quote_ata,
        ]: &[AccountInfo<'info>; SELL_ACCOUNTS_LEN3] =
            array_ref![accounts, offset, SELL_ACCOUNTS_LEN3];

        Ok(Self {
            dex_program_id,
            swap_authority_pubkey,
            swap_source_token: InterfaceAccount::try_from(swap_source_token)?,
            swap_destination_token: InterfaceAccount::try_from(swap_destination_token)?,
            pool,
            global_config,
            base_mint,
            quote_mint,
            pool_base_token_account,
            pool_quote_token_account,
            protocol_fee_recipient,
            protocol_fee_recipient_token_account,
            base_token_program,
            quote_token_program,
            system_program,
            associated_token_program,
            event_authority,
            coin_creator_vault_ata,
            coin_creator_vault_authority,
            fee_config,
            fee_program,
            pool_v2,
            buyback_fee_recipient,
            buyback_fee_recipient_quote_ata,
        })
    }
}
pub struct PumpfunammSellProcessor;
impl DexProcessor for PumpfunammSellProcessor {
    fn before_invoke(&self, account_infos: &[AccountInfo]) -> Result<u64> {
        Ok(account_infos.get(1).unwrap().lamports())
    }

    fn after_invoke(
        &self,
        account_infos: &[AccountInfo],
        _hop: usize,
        _owner_seeds: Option<&[&[&[u8]]]>,
        before_authority_lamports: u64,
    ) -> Result<u64> {
        let authority = account_infos.get(1).unwrap();
        if authority.key() == authority_pda::ID {
            // a first-use rent pump charged to the `user` slot during the
            // sell is absorbed by the authority's standing balance —
            // surfaced for ops, never reimbursed by the payer
            let diff_lamports =
                before_authority_lamports.saturating_sub(authority.lamports());
            if diff_lamports > 0 {
                require!(diff_lamports <= SOL_DIFF_LIMIT, ErrorCode::InvalidDiffLamports);
                msg!(
                    "pumpfunamm sell: authority bore {} lamports of pump first-use rent",
                    diff_lamports
                );
            }
        }
        Ok(0)
    }
}

pub fn sell3<'a>(
    remaining_accounts: &'a [AccountInfo<'a>],
    amount_in: u64,
    offset: &mut usize,
    hop_accounts: &mut HopAccounts,
    hop: usize,
    proxy_swap: bool,
    owner_seeds: Option<&[&[&[u8]]]>,
    payer: Option<&AccountInfo<'a>>,
) -> Result<u64> {
    msg!("Dex::Pumpfunamm amount_in: {}, offset: {}", amount_in, offset);
    require!(
        remaining_accounts.len() >= *offset + SELL_ACCOUNTS_LEN3,
        ErrorCode::InvalidAccountsLength
    );

    let mut swap_accounts: PumpfunammSellAccounts3<'_> =
        PumpfunammSellAccounts3::parse_accounts(remaining_accounts, *offset)?;
    if swap_accounts.dex_program_id.key != &pumpfunamm_program::id() {
        return Err(ErrorCode::InvalidProgramId.into());
    }
    // log pool address
    swap_accounts.pool.key().log();

    before_check(
        swap_accounts.swap_authority_pubkey,
        &swap_accounts.swap_source_token,
        swap_accounts.swap_destination_token.key(),
        hop_accounts,
        hop,
        proxy_swap,
        owner_seeds,
    )?;

    // amount_in base_mint_amount
    // amount_out quote_mint_amount
    let mut data = Vec::with_capacity(ARGS_LEN);
    data.extend_from_slice(PUMPFUN_SELL_SELECTOR);
    data.extend_from_slice(&amount_in.to_le_bytes()); // base_amount_in
    data.extend_from_slice(&1u64.to_le_bytes()); // min_quote_amount_out

    let mut accounts = Vec::with_capacity(21);
    accounts.push(AccountMeta::new(swap_accounts.pool.key(), false));
    accounts.push(AccountMeta::new(swap_accounts.swap_authority_pubkey.key(), true));
    accounts.push(AccountMeta::new_readonly(swap_accounts.global_config.key(), false));
    accounts.push(AccountMeta::new_readonly(swap_accounts.base_mint.key(), false));
    accounts.push(AccountMeta::new_readonly(swap_accounts.quote_mint.key(), false));
    accounts.push(AccountMeta::new(swap_accounts.swap_source_token.key(), false));
    accounts.push(AccountMeta::new(swap_accounts.swap_destination_token.key(), false));
    accounts.push(AccountMeta::new(swap_accounts.pool_base_token_account.key(), false));
    accounts.push(AccountMeta::new(swap_accounts.pool_quote_token_account.key(), false));
    accounts.push(AccountMeta::new_readonly(swap_accounts.protocol_fee_recipient.key(), false));
    accounts
        .push(AccountMeta::new(swap_accounts.protocol_fee_recipient_token_account.key(), false));
    accounts.push(AccountMeta::new_readonly(swap_accounts.base_token_program.key(), false));
    accounts.push(AccountMeta::new_readonly(swap_accounts.quote_token_program.key(), false));
    accounts.push(AccountMeta::new_readonly(swap_accounts.system_program.key(), false));
    accounts.push(AccountMeta::new_readonly(swap_accounts.associated_token_program.key(), false));
    accounts.push(AccountMeta::new_readonly(swap_accounts.event_authority.key(), false));
    accounts.push(AccountMeta::new_readonly(swap_accounts.dex_program_id.key(), false));
    accounts.push(AccountMeta::new(swap_accounts.coin_creator_vault_ata.key(), false));
    accounts
        .push(AccountMeta::new_readonly(swap_accounts.coin_creator_vault_authority.key(), false));
    accounts.push(AccountMeta::new_readonly(swap_accounts.fee_config.key(), false));
    accounts.push(AccountMeta::new_readonly(swap_accounts.fee_program.key(), false));
    if swap_accounts.pool_v2.key != &pumpfunamm_program::id() {
        accounts.push(AccountMeta::new_readonly(swap_accounts.pool_v2.key(), false));
    }
    accounts.push(AccountMeta::new_readonly(swap_accounts.buyback_fee_recipient.key(), false));
    accounts.push(AccountMeta::new(swap_accounts.buyback_fee_recipient_quote_ata.key(), false));

    let mut account_infos = arrayvec::ArrayVec::<_, { SELL_ACCOUNTS_LEN3 + 1 }>::new();
    account_infos.extend([
        swap_accounts.pool.to_account_info(),
        swap_accounts.swap_authority_pubkey.to_account_info(),
        swap_accounts.global_config.to_account_info(),
        swap_accounts.base_mint.to_account_info(),
        swap_accounts.quote_mint.to_account_info(),
        swap_accounts.swap_source_token.to_account_info(),
        swap_accounts.swap_destination_token.to_account_info(),
        swap_accounts.pool_base_token_account.to_account_info(),
        swap_accounts.pool_quote_token_account.to_account_info(),
        swap_accounts.protocol_fee_recipient.to_account_info(),
        swap_accounts.protocol_fee_recipient_token_account.to_account_info(),
        swap_accounts.base_token_program.to_account_info(),
        swap_accounts.quote_token_program.to_account_info(),
        swap_accounts.system_program.to_account_info(),
        swap_accounts.associated_token_program.to_account_info(),
        swap_accounts.event_authority.to_account_info(),
        swap_accounts.dex_program_id.to_account_info(),
        swap_accounts.coin_creator_vault_ata.to_account_info(),
        swap_accounts.coin_creator_vault_authority.to_account_info(),
        swap_accounts.fee_config.to_account_info(),
        swap_accounts.fee_program.to_account_info(),
    ]);
    if swap_accounts.pool_v2.key != &pumpfunamm_program::id() {
        account_infos.push(swap_accounts.pool_v2.to_account_info());
    }
    account_infos.push(swap_accounts.buyback_fee_recipient.to_account_info());
    account_infos.push(swap_accounts.buyback_fee_recipient_quote_ata.to_account_info());
    account_infos.push(payer.unwrap().to_account_info());

    let instruction =
        Instruction { program_id: swap_accounts.dex_program_id.key(), accounts, data };

    let dex_processor = &PumpfunammSellProcessor;
    let amount_out = invoke_process(
        amount_in,
        dex_processor,
        &account_infos,
        &mut swap_accounts.swap_source_token,
        &mut swap_accounts.swap_destination_token,
        hop_accounts,
        instruction,
        hop,
        offset,
        SELL_ACCOUNTS_LEN3,
        proxy_swap,
        owner_seeds,
    )?;

    Ok(amount_out)
}
pub struct PumpfunammBuyAccounts3<'info> {
    pub dex_program_id: &'info AccountInfo<'info>,
    pub swap_authority_pubkey: &'info AccountInfo<'info>,
    pub swap_source_token: InterfaceAccount<'info, TokenAccount>,
    pub swap_destination_token: InterfaceAccount<'info, TokenAccount>,

    pub pool: &'info AccountInfo<'info>,
    pub global_config: &'info AccountInfo<'info>,
    pub base_mint: &'info AccountInfo<'info>,
    pub quote_mint: &'info AccountInfo<'info>,
    pub pool_base_token_account: &'info AccountInfo<'info>,
    pub pool_quote_token_account: &'info AccountInfo<'info>,
    pub protocol_fee_recipient: &'info AccountInfo<'info>,
    pub protocol_fee_recipient_token_account: &'info AccountInfo<'info>,
    pub base_token_program: &'info AccountInfo<'info>,
    pub quote_token_program: &'info AccountInfo<'info>,
    pub system_program: &'info AccountInfo<'info>,
    pub associated_token_program: &'info AccountInfo<'info>,
    pub event_authority: &'info AccountInfo<'info>,
    pub coin_creator_vault_ata: &'info AccountInfo<'info>,
    pub coin_creator_vault_authority: &'info AccountInfo<'info>,
    pub global_volume_accumulator: &'info AccountInfo<'info>,
    pub user_volume_accumulator: &'info AccountInfo<'info>,
    pub fee_config: &'info AccountInfo<'info>,
    pub fee_program: &'info AccountInfo<'info>,
    // same trailing trio as sell (see PumpfunammSellAccounts3)
    pub pool_v2: &'info AccountInfo<'info>,
    pub buyback_fee_recipient: &'info AccountInfo<'info>,
    pub buyback_fee_recipient_quote_ata: &'info AccountInfo<'info>,
}
const BUY_ACCOUNTS_LEN3: usize = 26;

// Fixed indices into the buy adapter's account_infos. pool_v2 is appended
// after these slots, so both stay stable whether or not the pool carries it;
// the payer rides last (see buy3).
const BUY_AUTHORITY_IDX: usize = 1;
const BUY_USER_VOLUME_ACC_IDX: usize = 20;
// hop0 drained-temp close: the quote-side source account and its token
// program in the processor row (the close needs the program CPI)
const BUY_SRC_IDX: usize = 6;
const BUY_DST_IDX: usize = 5;
const BUY_QUOTE_TOKEN_PROGRAM_IDX: usize = 12;

impl<'info> PumpfunammBuyAccounts3<'info> {
    fn parse_accounts(accounts: &'info [AccountInfo<'info>], offset: usize) -> Result<Self> {
        let [
            dex_program_id,
            swap_authority_pubkey,
            swap_source_token,
            swap_destination_token,
            pool,
            global_config,
            base_mint,
            quote_mint,
            pool_base_token_account,
            pool_quote_token_account,
            protocol_fee_recipient,
            protocol_fee_recipient_token_account,
            base_token_program,
            quote_token_program,
            system_program,
            associated_token_program,
            event_authority,
            coin_creator_vault_ata,
            coin_creator_vault_authority,
            global_volume_accumulator,
            user_volume_accumulator,
            fee_config,
            fee_program,
            pool_v2,
            buyback_fee_recipient,
            buyback_fee_recipient_quote_ata,
        ]: &[AccountInfo<'info>; BUY_ACCOUNTS_LEN3] =
            array_ref![accounts, offset, BUY_ACCOUNTS_LEN3];

        Ok(Self {
            dex_program_id,
            swap_authority_pubkey,
            swap_source_token: InterfaceAccount::try_from(swap_source_token)?,
            swap_destination_token: InterfaceAccount::try_from(swap_destination_token)?,
            pool,
            global_config,
            base_mint,
            quote_mint,
            pool_base_token_account,
            pool_quote_token_account,
            protocol_fee_recipient,
            protocol_fee_recipient_token_account,
            base_token_program,
            quote_token_program,
            system_program,
            associated_token_program,
            event_authority,
            coin_creator_vault_ata,
            coin_creator_vault_authority,
            global_volume_accumulator,
            user_volume_accumulator,
            fee_config,
            fee_program,
            pool_v2,
            buyback_fee_recipient,
            buyback_fee_recipient_quote_ata,
        })
    }
}
pub struct PumpfunammBuyProcessor;
impl DexProcessor for PumpfunammBuyProcessor {
    fn before_invoke(&self, account_infos: &[AccountInfo]) -> Result<u64> {
        // the pump_amm `user` is the authority PDA at every hop (2026-10-22
        // unification): its standing balance (seeded by ops) bears pump's
        // first-use rents, and the wallet never becomes a pump user
        let authority = account_infos.get(BUY_AUTHORITY_IDX).unwrap();
        let before_authority_lamports = authority.lamports();

        // Custody topology: the authority PDA sits in pump's `user` slot,
        // so the first-ever buy makes pump create the per-user volume
        // accumulator and charge its rent to the PDA. Fail fast while the
        // accumulator is unborn and the balance can't cover the rent — the
        // payer is never charged, and steady-state buys (accumulator
        // exists) move no native lamports at all.
        let user_volume_accumulator = account_infos.get(BUY_USER_VOLUME_ACC_IDX).unwrap();
        let accumulator_missing =
            user_volume_accumulator.lamports() == 0 && user_volume_accumulator.data_len() == 0;
        if accumulator_missing {
            require!(
                before_authority_lamports >= PUMPFUNAMM_FUND_HEADROOM,
                ErrorCode::InsufficientFunds
            );
        }
        // defense in depth (the route builder is permissionless): no
        // PDA-owned token account may ride this CPI outside the hop's
        // declared source/destination slots
        enforce_sa_token_allowlist(
            account_infos,
            &[
                account_infos.get(BUY_SRC_IDX).unwrap().key(),
                account_infos.get(BUY_DST_IDX).unwrap().key(),
            ],
        )?;
        Ok(before_authority_lamports)
    }

    fn after_invoke(
        &self,
        account_infos: &[AccountInfo],
        hop: usize,
        _owner_seeds: Option<&[&[&[u8]]]>,
        before_authority_lamports: u64,
    ) -> Result<u64> {
        let authority = account_infos.get(BUY_AUTHORITY_IDX).unwrap();
        // hop0: the source is a disposable PDA-owned temp (custody only
        // enters at hop >= 1 — compose guarantees it, same contract as the
        // curve adapter's forced temp); pump has now debited it exactly,
        // so close it and return the fronted rent to the payer. The shared
        // wSOL account is hard-blocked as a belt-and-suspenders guard:
        // the route builder is permissionless, so per-tx temps are the
        // only accounts that may ever ride this close.
        if hop == 0 {
            let source = account_infos.get(BUY_SRC_IDX).unwrap();
            require!(
                source.key() != wsol_sa::ID,
                ErrorCode::InvalidSourceTokenAccount
            );
            let payer = account_infos.last().unwrap();
            let quote_token_program = account_infos.get(BUY_QUOTE_TOKEN_PROGRAM_IDX).unwrap();
            close_token_account(
                source.to_account_info(),
                payer.to_account_info(),
                authority.to_account_info(),
                quote_token_program.to_account_info(),
                Some(SA_AUTHORITY_SEED),
            )?;
        }
        // Settle any surplus back to the payer; a deficit is pump's
        // first-use rent charged to the `user` slot, absorbed by the
        // authority's standing balance — the payer is never charged.
        let drain = authority
            .lamports()
            .saturating_sub(before_authority_lamports);
        require!(drain <= SOL_DIFF_LIMIT, ErrorCode::InvalidDiffLamports);
        if drain > 0 {
            let payer = account_infos.last().unwrap();
            transfer_sol(
                authority.to_account_info(),
                payer.to_account_info(),
                drain,
                Some(SA_AUTHORITY_SEED),
            )?;
            msg!(
                "pumpfunamm buy: authority swept {} lamports back to payer",
                drain
            );
        }
        Ok(0)
    }
}

pub fn buy3<'a>(
    remaining_accounts: &'a [AccountInfo<'a>],
    amount_in: u64,
    offset: &mut usize,
    hop_accounts: &mut HopAccounts,
    hop: usize,
    proxy_swap: bool,
    owner_seeds: Option<&[&[&[u8]]]>,
    payer: Option<&AccountInfo<'a>>,
) -> Result<u64> {
    msg!("Dex::Pumpfunamm amount_in: {}, offset: {}", amount_in, offset);
    require!(
        remaining_accounts.len() >= *offset + BUY_ACCOUNTS_LEN3,
        ErrorCode::InvalidAccountsLength
    );

    let mut swap_accounts = PumpfunammBuyAccounts3::parse_accounts(remaining_accounts, *offset)?;
    if swap_accounts.dex_program_id.key != &pumpfunamm_program::id() {
        return Err(ErrorCode::InvalidProgramId.into());
    }
    // log pool address
    swap_accounts.pool.key().log();

    // the pump_amm `user` is the authority PDA at every hop (2026-10-22
    // unification): wallets never become pump users and never pay the
    // per-user first-use rents themselves
    before_check_pda_user(
        swap_accounts.swap_authority_pubkey,
        &swap_accounts.swap_source_token,
        swap_accounts.swap_destination_token.key(),
        hop_accounts,
    )?;

    let mut data = Vec::with_capacity(ARGS_LEN);
    data.extend_from_slice(BUY_EXACT_QUOTE_IN_SELECTOR);
    data.extend_from_slice(&amount_in.to_le_bytes()); // spendable_quote_in
    data.extend_from_slice(&1u64.to_le_bytes()); // min_base_amount_out
    // track_volume: OptionBool { true } — pump_amm added this arg after the
    // upstream snapshot; without the byte the on-chain borsh deserialization
    // of buy_exact_quote_in fails (verified against the live IDL, 2026-10)
    data.extend_from_slice(&1u8.to_le_bytes());

    let mut accounts = Vec::with_capacity(23);
    accounts.push(AccountMeta::new(swap_accounts.pool.key(), false));
    accounts.push(AccountMeta::new(swap_accounts.swap_authority_pubkey.key(), true));
    accounts.push(AccountMeta::new_readonly(swap_accounts.global_config.key(), false));
    accounts.push(AccountMeta::new_readonly(swap_accounts.base_mint.key(), false));
    accounts.push(
        // wsol
        AccountMeta::new_readonly(swap_accounts.quote_mint.key(), false),
    );
    accounts.push(
        // usdc
        AccountMeta::new(swap_accounts.swap_destination_token.key(), false),
    );
    accounts.push(
        // wsol-ata
        AccountMeta::new(swap_accounts.swap_source_token.key(), false),
    );
    accounts.push(
        // usdc-ata
        AccountMeta::new(swap_accounts.pool_base_token_account.key(), false),
    );
    accounts.push(
        // wsol-ata
        AccountMeta::new(swap_accounts.pool_quote_token_account.key(), false),
    );
    accounts.push(
        //usdc-ata
        AccountMeta::new_readonly(swap_accounts.protocol_fee_recipient.key(), false),
    );
    accounts
        .push(AccountMeta::new(swap_accounts.protocol_fee_recipient_token_account.key(), false));
    accounts.push(AccountMeta::new_readonly(swap_accounts.base_token_program.key(), false));
    accounts.push(AccountMeta::new_readonly(swap_accounts.quote_token_program.key(), false));
    accounts.push(AccountMeta::new_readonly(swap_accounts.system_program.key(), false));
    accounts.push(AccountMeta::new_readonly(swap_accounts.associated_token_program.key(), false));
    accounts.push(AccountMeta::new_readonly(swap_accounts.event_authority.key(), false));
    accounts.push(AccountMeta::new_readonly(swap_accounts.dex_program_id.key(), false));
    accounts.push(AccountMeta::new(swap_accounts.coin_creator_vault_ata.key(), false));
    accounts
        .push(AccountMeta::new_readonly(swap_accounts.coin_creator_vault_authority.key(), false));
    accounts.push(AccountMeta::new(swap_accounts.global_volume_accumulator.key(), false));
    accounts.push(AccountMeta::new(swap_accounts.user_volume_accumulator.key(), false));
    accounts.push(AccountMeta::new_readonly(swap_accounts.fee_config.key(), false));
    accounts.push(AccountMeta::new_readonly(swap_accounts.fee_program.key(), false));
    if swap_accounts.pool_v2.key != &pumpfunamm_program::id() {
        accounts.push(AccountMeta::new_readonly(swap_accounts.pool_v2.key(), false));
    }
    accounts.push(AccountMeta::new_readonly(swap_accounts.buyback_fee_recipient.key(), false));
    accounts.push(AccountMeta::new(swap_accounts.buyback_fee_recipient_quote_ata.key(), false));

    let mut account_infos = arrayvec::ArrayVec::<_, { BUY_ACCOUNTS_LEN3 + 1 }>::new();
    account_infos.push(swap_accounts.pool.to_account_info());
    account_infos.push(swap_accounts.swap_authority_pubkey.to_account_info());
    account_infos.push(swap_accounts.global_config.to_account_info());
    account_infos.push(swap_accounts.base_mint.to_account_info());
    account_infos.push(swap_accounts.quote_mint.to_account_info());
    account_infos.push(swap_accounts.swap_destination_token.to_account_info());
    account_infos.push(swap_accounts.swap_source_token.to_account_info());
    account_infos.push(swap_accounts.pool_base_token_account.to_account_info());
    account_infos.push(swap_accounts.pool_quote_token_account.to_account_info());
    account_infos.push(swap_accounts.protocol_fee_recipient.to_account_info());
    account_infos.push(swap_accounts.protocol_fee_recipient_token_account.to_account_info());
    account_infos.push(swap_accounts.base_token_program.to_account_info());
    account_infos.push(swap_accounts.quote_token_program.to_account_info());
    account_infos.push(swap_accounts.system_program.to_account_info());
    account_infos.push(swap_accounts.associated_token_program.to_account_info());
    account_infos.push(swap_accounts.event_authority.to_account_info());
    account_infos.push(swap_accounts.dex_program_id.to_account_info());
    account_infos.push(swap_accounts.coin_creator_vault_ata.to_account_info());
    account_infos.push(swap_accounts.coin_creator_vault_authority.to_account_info());
    account_infos.push(swap_accounts.global_volume_accumulator.to_account_info());
    account_infos.push(swap_accounts.user_volume_accumulator.to_account_info());
    account_infos.push(swap_accounts.fee_config.to_account_info());
    account_infos.push(swap_accounts.fee_program.to_account_info());
    if swap_accounts.pool_v2.key != &pumpfunamm_program::id() {
        account_infos.push(swap_accounts.pool_v2.to_account_info());
    }
    account_infos.push(swap_accounts.buyback_fee_recipient.to_account_info());
    account_infos.push(swap_accounts.buyback_fee_recipient_quote_ata.to_account_info());
    // payer rides last: the after-hook returns the hop0 temp's rent and
    // any surplus to it — see PumpfunammBuyProcessor
    account_infos.push(payer.unwrap().to_account_info());

    let instruction =
        Instruction { program_id: swap_accounts.dex_program_id.key(), accounts, data };

    let dex_processor = &PumpfunammBuyProcessor;
    let amount_out = invoke_process(
        amount_in,
        dex_processor,
        &account_infos,
        &mut swap_accounts.swap_source_token,
        &mut swap_accounts.swap_destination_token,
        hop_accounts,
        instruction,
        hop,
        offset,
        BUY_ACCOUNTS_LEN3,
        proxy_swap,
        owner_seeds,
    )?;

    Ok(amount_out)
}

/*============================= pumpfunamm abort function ============================= */
pub fn sell<'a>(
    _remaining_accounts: &'a [AccountInfo<'a>],
    _amount_in: u64,
    _offset: &mut usize,
    _hop_accounts: &mut HopAccounts,
    _hop: usize,
    _proxy_swap: bool,
    _owner_seeds: Option<&[&[&[u8]]]>,
) -> Result<u64> {
    msg!("Dex::Pumpfunamm ABORT");
    require!(true == false, ErrorCode::AdapterAbort);
    Ok(0)
}

pub fn buy<'a>(
    _remaining_accounts: &'a [AccountInfo<'a>],
    _amount_in: u64,
    _offset: &mut usize,
    _hop_accounts: &mut HopAccounts,
    _hop: usize,
    _proxy_swap: bool,
    _owner_seeds: Option<&[&[&[u8]]]>,
) -> Result<u64> {
    msg!("Dex::Pumpfunamm ABORT");
    require!(true == false, ErrorCode::AdapterAbort);
    Ok(0)
}

pub fn buy2<'a>(
    _remaining_accounts: &'a [AccountInfo<'a>],
    _amount_in: u64,
    _offset: &mut usize,
    _hop_accounts: &mut HopAccounts,
    _hop: usize,
    _proxy_swap: bool,
    _owner_seeds: Option<&[&[&[u8]]]>,
) -> Result<u64> {
    msg!("Dex::Pumpfunamm ABORT");
    require!(true == false, ErrorCode::AdapterAbort);
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    // The processor-row indexes pin the buy3 account_infos push order
    // (pool, authority, global_config, base_mint, quote_mint, dst, src,
    // pool_base, pool_quote, fee_recipient, fee_recipient_ata, base_prog,
    // quote_prog, system, ata, event, dex, vault_ata, vault_auth, gva,
    // uva, fee_config, fee_program, [pool_v2], buyback, buyback_ata,
    // payer-last); pool_v2 inserts at 23 so these all stay stable.
    #[test]
    pub fn test_buy3_processor_indexes() {
        assert_eq!(BUY_AUTHORITY_IDX, 1);
        assert_eq!(BUY_DST_IDX, 5);
        assert_eq!(BUY_SRC_IDX, 6);
        assert_eq!(BUY_QUOTE_TOKEN_PROGRAM_IDX, 12);
        assert_eq!(BUY_USER_VOLUME_ACC_IDX, 20);
    }
}
