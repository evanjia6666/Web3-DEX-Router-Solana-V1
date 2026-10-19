use super::common::{before_check, invoke_process, DexProcessor};
use crate::error::ErrorCode;
use crate::utils::{close_token_account, sync_wsol_account, transfer_sol};
use crate::{
    HopAccounts, BUY_EXACT_SOL_IN_SELECTOR, MIN_SOL_ACCOUNT_RENT,
    PUMPFUN_BUY_EXACT_QUOTE_IN_V2_SELECTOR, PUMPFUN_FUND_HEADROOM,
    PUMPFUN_SELL_V2_SELECTOR, PUMPFUN_SELL_SELECTOR, PUMPFUN_V2_FUND_HEADROOM, SOL_DIFF_LIMIT,
    SA_AUTHORITY_SEED, authority_pda, pumpfun_program, wsol_sa,
};
use anchor_lang::prelude::*;
use anchor_lang::solana_program::instruction::Instruction;
use anchor_spl::token_interface::TokenAccount;
use arrayref::array_ref;

// pump.fun bonding-curve adapter (Dex::PumpfunBuy / Dex::PumpfunSell).
//
// The curve trades SOL natively: `buy_exact_sol_in` spends native SOL from
// the `user` account (fees deducted inside the spendable amount) and `sell`
// pays native SOL back to it. The router's hop model moves value between
// token accounts, so both directions bridge through the swap authority's
// native balance:
//
//   buy  — before the CPI the (disposable, never custody) wSOL source
//          account is closed into the authority, releasing rent + wrapped
//          input as native SOL; on the PDA the payer tops up any shortfall
//          against pump's on-first-use rents, and after the CPI everything
//          above the rent floor is swept back to the payer.
//   sell — the CPI pays the authority natively; after it the exact received
//          delta is transferred into the wSOL destination account and
//          sync_native'd, restoring the token-account settlement model.
//
// Wire shape (verified by mainnet simulation 2026-10): buy carries the 16
// declared accounts plus two mandatory trailing buyback slots (the second
// must be an authorized pump-fees vault; the first is not validated), sell
// needs its 14 declared accounts only. track_volume (OptionBool) is omitted
// from buy_exact_sol_in — anchor deserializes the missing byte as None, so
// no user volume accumulator is created and its rent never applies.
const ARGS_LEN: usize = 24;

/*===================================== buy =====================================*/

pub struct PumpfunBuyAccounts<'info> {
    pub dex_program_id: &'info AccountInfo<'info>,
    pub swap_authority_pubkey: &'info AccountInfo<'info>,
    pub swap_source_token: InterfaceAccount<'info, TokenAccount>,
    pub swap_destination_token: InterfaceAccount<'info, TokenAccount>,
    pub global: &'info AccountInfo<'info>,
    pub fee_recipient: &'info AccountInfo<'info>,
    pub mint: &'info AccountInfo<'info>,
    pub bonding_curve: &'info AccountInfo<'info>,
    pub associated_bonding_curve: &'info AccountInfo<'info>,
    pub system_program: &'info AccountInfo<'info>,
    // SPL Token — owns the wSOL source account; distinct from the base
    // mint's program because pump coins are frequently Token-2022
    pub src_token_program: &'info AccountInfo<'info>,
    // the base mint's token program, forwarded to the curve CPI
    pub base_token_program: &'info AccountInfo<'info>,
    pub creator_vault: &'info AccountInfo<'info>,
    pub event_authority: &'info AccountInfo<'info>,
    pub global_volume_accumulator: &'info AccountInfo<'info>,
    pub user_volume_accumulator: &'info AccountInfo<'info>,
    pub fee_config: &'info AccountInfo<'info>,
    pub fee_program: &'info AccountInfo<'info>,
    pub buyback_a: &'info AccountInfo<'info>,
    pub buyback_b: &'info AccountInfo<'info>,
}
// remaining-accounts window: the 20 above
const BUY_ACCOUNTS_LEN: usize = 20;
// account_infos handed to the processor: the 18 CPI accounts, then the
// close/sweep helpers (src program, payer, src)
const BUY_PROCESSOR_LEN: usize = 21;

impl<'info> PumpfunBuyAccounts<'info> {
    fn parse_accounts(accounts: &'info [AccountInfo<'info>], offset: usize) -> Result<Self> {
        let [
            dex_program_id,
            swap_authority_pubkey,
            swap_source_token,
            swap_destination_token,
            global,
            fee_recipient,
            mint,
            bonding_curve,
            associated_bonding_curve,
            system_program,
            src_token_program,
            base_token_program,
            creator_vault,
            event_authority,
            global_volume_accumulator,
            user_volume_accumulator,
            fee_config,
            fee_program,
            buyback_a,
            buyback_b,
        ]: &[AccountInfo<'info>; BUY_ACCOUNTS_LEN] =
            array_ref![accounts, offset, BUY_ACCOUNTS_LEN];

        Ok(Self {
            dex_program_id,
            swap_authority_pubkey,
            swap_source_token: InterfaceAccount::try_from(swap_source_token)?,
            swap_destination_token: InterfaceAccount::try_from(swap_destination_token)?,
            global,
            fee_recipient,
            mint,
            bonding_curve,
            associated_bonding_curve,
            system_program,
            src_token_program,
            base_token_program,
            creator_vault,
            event_authority,
            global_volume_accumulator,
            user_volume_accumulator,
            fee_config,
            fee_program,
            buyback_a,
            buyback_b,
        })
    }
}

pub struct PumpfunBuyProcessor {
    pub spendable: u64,
}

impl DexProcessor for PumpfunBuyProcessor {
    fn before_invoke(&self, account_infos: &[AccountInfo]) -> Result<u64> {
        let authority = account_infos.get(6).unwrap();
        let src_token_program = account_infos.get(BUY_PROCESSOR_LEN - 3).unwrap();
        let payer = account_infos.get(BUY_PROCESSOR_LEN - 2).unwrap();
        let source_token_account = account_infos.last().unwrap();
        let before_authority_lamports = authority.lamports();

        if authority.key() == authority_pda::ID {
            // the source is closed into the authority: it must be a
            // disposable per-tx wSOL temp, never the shared wsol_sa or a
            // warmed custody account (the caller guarantees this; the
            // shared account would be destroyed)
            require!(
                source_token_account.key() != wsol_sa::ID,
                ErrorCode::InvalidSourceTokenAccount
            );
            // The closed temp carries rent + wrapped input; pump may still
            // need more than that for its on-first-use rents, so the payer
            // covers any shortfall up front and the after-hook sweeps the
            // residue back — net zero for everyone but pump.
            let needed = self
                .spendable
                .checked_add(PUMPFUN_FUND_HEADROOM)
                .ok_or(ErrorCode::CalculationError)?;
            let have = before_authority_lamports
                .checked_add(source_token_account.get_lamports())
                .ok_or(ErrorCode::CalculationError)?;
            if have < needed {
                transfer_sol(
                    payer.to_account_info(),
                    authority.to_account_info(),
                    needed - have,
                    None,
                )?;
            }
            close_token_account(
                source_token_account.to_account_info(),
                authority.to_account_info(),
                authority.to_account_info(),
                src_token_program.to_account_info(),
                Some(SA_AUTHORITY_SEED),
            )?;
        } else {
            // hop0 direct: the wallet owns the temp and signs the tx; the
            // close simply returns the wrapped input to the wallet
            close_token_account(
                source_token_account.to_account_info(),
                authority.to_account_info(),
                authority.to_account_info(),
                src_token_program.to_account_info(),
                None,
            )?;
        }
        Ok(before_authority_lamports)
    }

    fn after_invoke(
        &self,
        account_infos: &[AccountInfo],
        _hop: usize,
        _owner_seeds: Option<&[&[&[u8]]]>,
        _before_authority_lamports: u64,
    ) -> Result<u64> {
        let authority = account_infos.get(6).unwrap();
        let payer = account_infos.get(BUY_PROCESSOR_LEN - 2).unwrap();
        if authority.key() == authority_pda::ID {
            // sweep everything above the rent floor back to the payer,
            // leaving the PDA rent-exempt for the next swap
            let drain = authority.lamports().saturating_sub(MIN_SOL_ACCOUNT_RENT);
            require!(drain <= SOL_DIFF_LIMIT, ErrorCode::InvalidDiffLamports);
            transfer_sol(
                authority.to_account_info(),
                payer.to_account_info(),
                drain,
                Some(SA_AUTHORITY_SEED),
            )?;
            msg!(
                "pumpfun buy: authority swept {} lamports back to payer",
                drain
            );
        }
        Ok(0)
    }
}

#[allow(clippy::too_many_arguments)]
pub fn buy<'a>(
    remaining_accounts: &'a [AccountInfo<'a>],
    amount_in: u64,
    offset: &mut usize,
    hop_accounts: &mut HopAccounts,
    hop: usize,
    proxy_swap: bool,
    owner_seeds: Option<&[&[&[u8]]]>,
    payer: Option<&AccountInfo<'a>>,
) -> Result<u64> {
    msg!("Dex::PumpfunBuy amount_in: {}, offset: {}", amount_in, offset);
    require!(
        remaining_accounts.len() >= *offset + BUY_ACCOUNTS_LEN,
        ErrorCode::InvalidAccountsLength
    );
    let mut swap_accounts = PumpfunBuyAccounts::parse_accounts(remaining_accounts, *offset)?;
    if swap_accounts.dex_program_id.key != &pumpfun_program::id() {
        return Err(ErrorCode::InvalidProgramId.into());
    }
    swap_accounts.bonding_curve.key().log();

    before_check(
        swap_accounts.swap_authority_pubkey,
        &swap_accounts.swap_source_token,
        swap_accounts.swap_destination_token.key(),
        hop_accounts,
        hop,
        proxy_swap,
        owner_seeds,
    )?;

    // spendable_sol_in = amount_in (fees inside), min_tokens_out = 1: the
    // route-level min_return bound already protects this leg
    let mut data = Vec::with_capacity(ARGS_LEN);
    data.extend_from_slice(BUY_EXACT_SOL_IN_SELECTOR);
    data.extend_from_slice(&amount_in.to_le_bytes());
    data.extend_from_slice(&1u64.to_le_bytes());

    let mut accounts = Vec::with_capacity(18);
    accounts.push(AccountMeta::new_readonly(swap_accounts.global.key(), false));
    accounts.push(AccountMeta::new(swap_accounts.fee_recipient.key(), false));
    accounts.push(AccountMeta::new_readonly(swap_accounts.mint.key(), false));
    accounts.push(AccountMeta::new(swap_accounts.bonding_curve.key(), false));
    accounts.push(AccountMeta::new(swap_accounts.associated_bonding_curve.key(), false));
    accounts.push(AccountMeta::new(swap_accounts.swap_destination_token.key(), false));
    accounts.push(AccountMeta::new(swap_accounts.swap_authority_pubkey.key(), true));
    accounts.push(AccountMeta::new_readonly(swap_accounts.system_program.key(), false));
    accounts.push(AccountMeta::new_readonly(swap_accounts.base_token_program.key(), false));
    accounts.push(AccountMeta::new(swap_accounts.creator_vault.key(), false));
    accounts.push(AccountMeta::new_readonly(swap_accounts.event_authority.key(), false));
    accounts.push(AccountMeta::new_readonly(swap_accounts.dex_program_id.key(), false));
    accounts.push(AccountMeta::new_readonly(swap_accounts.global_volume_accumulator.key(), false));
    accounts.push(AccountMeta::new(swap_accounts.user_volume_accumulator.key(), false));
    accounts.push(AccountMeta::new_readonly(swap_accounts.fee_config.key(), false));
    accounts.push(AccountMeta::new_readonly(swap_accounts.fee_program.key(), false));
    accounts.push(AccountMeta::new(swap_accounts.buyback_a.key(), false));
    accounts.push(AccountMeta::new(swap_accounts.buyback_b.key(), false));

    let account_infos = [
        swap_accounts.global.to_account_info(),
        swap_accounts.fee_recipient.to_account_info(),
        swap_accounts.mint.to_account_info(),
        swap_accounts.bonding_curve.to_account_info(),
        swap_accounts.associated_bonding_curve.to_account_info(),
        swap_accounts.swap_destination_token.to_account_info(),
        swap_accounts.swap_authority_pubkey.to_account_info(),
        swap_accounts.system_program.to_account_info(),
        swap_accounts.base_token_program.to_account_info(),
        swap_accounts.creator_vault.to_account_info(),
        swap_accounts.event_authority.to_account_info(),
        swap_accounts.dex_program_id.to_account_info(),
        swap_accounts.global_volume_accumulator.to_account_info(),
        swap_accounts.user_volume_accumulator.to_account_info(),
        swap_accounts.fee_config.to_account_info(),
        swap_accounts.fee_program.to_account_info(),
        swap_accounts.buyback_a.to_account_info(),
        swap_accounts.buyback_b.to_account_info(),
        swap_accounts.src_token_program.to_account_info(),
        payer.unwrap().to_account_info(),
        swap_accounts.swap_source_token.to_account_info(),
    ];

    let instruction =
        Instruction { program_id: swap_accounts.dex_program_id.key(), accounts, data };

    let dex_processor = &PumpfunBuyProcessor { spendable: amount_in };
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
        BUY_ACCOUNTS_LEN,
        proxy_swap,
        owner_seeds,
    )?;
    Ok(amount_out)
}

/*===================================== sell ==================================== */

pub struct PumpfunSellAccounts<'info> {
    pub dex_program_id: &'info AccountInfo<'info>,
    pub swap_authority_pubkey: &'info AccountInfo<'info>,
    pub swap_source_token: InterfaceAccount<'info, TokenAccount>,
    pub swap_destination_token: InterfaceAccount<'info, TokenAccount>,
    pub global: &'info AccountInfo<'info>,
    pub fee_recipient: &'info AccountInfo<'info>,
    pub mint: &'info AccountInfo<'info>,
    pub bonding_curve: &'info AccountInfo<'info>,
    pub associated_bonding_curve: &'info AccountInfo<'info>,
    pub system_program: &'info AccountInfo<'info>,
    pub creator_vault: &'info AccountInfo<'info>,
    pub base_token_program: &'info AccountInfo<'info>,
    // SPL Token — owns the wSOL destination the native proceeds wrap into
    pub wsol_program: &'info AccountInfo<'info>,
    pub event_authority: &'info AccountInfo<'info>,
    pub fee_config: &'info AccountInfo<'info>,
    pub fee_program: &'info AccountInfo<'info>,
}
// remaining-accounts window: the 16 above
const SELL_ACCOUNTS_LEN: usize = 16;

impl<'info> PumpfunSellAccounts<'info> {
    fn parse_accounts(accounts: &'info [AccountInfo<'info>], offset: usize) -> Result<Self> {
        let [
            dex_program_id,
            swap_authority_pubkey,
            swap_source_token,
            swap_destination_token,
            global,
            fee_recipient,
            mint,
            bonding_curve,
            associated_bonding_curve,
            system_program,
            creator_vault,
            base_token_program,
            wsol_program,
            event_authority,
            fee_config,
            fee_program,
        ]: &[AccountInfo<'info>; SELL_ACCOUNTS_LEN] =
            array_ref![accounts, offset, SELL_ACCOUNTS_LEN];

        Ok(Self {
            dex_program_id,
            swap_authority_pubkey,
            swap_source_token: InterfaceAccount::try_from(swap_source_token)?,
            swap_destination_token: InterfaceAccount::try_from(swap_destination_token)?,
            global,
            fee_recipient,
            mint,
            bonding_curve,
            associated_bonding_curve,
            system_program,
            creator_vault,
            base_token_program,
            wsol_program,
            event_authority,
            fee_config,
            fee_program,
        })
    }
}

pub struct PumpfunSellProcessor;

impl DexProcessor for PumpfunSellProcessor {
    fn before_invoke(&self, account_infos: &[AccountInfo]) -> Result<u64> {
        // lamports before, for both authority kinds: the sell pays the
        // authority natively and the after-hook wraps exactly the delta
        Ok(account_infos.get(6).unwrap().lamports())
    }

    fn after_invoke(
        &self,
        account_infos: &[AccountInfo],
        hop: usize,
        owner_seeds: Option<&[&[&[u8]]]>,
        before_authority_lamports: u64,
    ) -> Result<u64> {
        let authority = account_infos.get(6).unwrap();
        let wsol_program = account_infos.get(SELL_PROCESSOR_LEN - 2).unwrap();
        let destination_token_account = account_infos.last().unwrap();

        let signer_seeds: Option<&[&[&[u8]]]> = if authority.key() == authority_pda::ID {
            Some(SA_AUTHORITY_SEED)
        } else if hop == 0 && owner_seeds.is_some() {
            Some(owner_seeds.unwrap())
        } else {
            None
        };

        let received = authority
            .lamports()
            .checked_sub(before_authority_lamports)
            .ok_or(ErrorCode::CalculationError)?;
        require!(received > 0, ErrorCode::AmountOutMustBeGreaterThanZero);
        transfer_sol(
            authority.to_account_info(),
            destination_token_account.to_account_info(),
            received,
            signer_seeds,
        )?;
        sync_wsol_account(
            destination_token_account.to_account_info(),
            wsol_program.to_account_info(),
            signer_seeds,
        )?;
        msg!("pumpfun sell: wrapped {} lamports into destination", received);
        Ok(received)
    }
}
// account_infos handed to the processor: the 14 CPI accounts plus the two
// wrap helpers (wsol program, destination). The sell moves no payer funds,
// so unlike buy there is no payer slot.
const SELL_PROCESSOR_LEN: usize = 16;

#[allow(clippy::too_many_arguments)]
pub fn sell<'a>(
    remaining_accounts: &'a [AccountInfo<'a>],
    amount_in: u64,
    offset: &mut usize,
    hop_accounts: &mut HopAccounts,
    hop: usize,
    proxy_swap: bool,
    owner_seeds: Option<&[&[&[u8]]]>,
    _payer: Option<&AccountInfo<'a>>,
) -> Result<u64> {
    msg!("Dex::PumpfunSell amount_in: {}, offset: {}", amount_in, offset);
    require!(
        remaining_accounts.len() >= *offset + SELL_ACCOUNTS_LEN,
        ErrorCode::InvalidAccountsLength
    );
    let mut swap_accounts = PumpfunSellAccounts::parse_accounts(remaining_accounts, *offset)?;
    if swap_accounts.dex_program_id.key != &pumpfun_program::id() {
        return Err(ErrorCode::InvalidProgramId.into());
    }
    swap_accounts.bonding_curve.key().log();

    before_check(
        swap_accounts.swap_authority_pubkey,
        &swap_accounts.swap_source_token,
        swap_accounts.swap_destination_token.key(),
        hop_accounts,
        hop,
        proxy_swap,
        owner_seeds,
    )?;

    let mut data = Vec::with_capacity(ARGS_LEN);
    data.extend_from_slice(PUMPFUN_SELL_SELECTOR);
    data.extend_from_slice(&amount_in.to_le_bytes());
    data.extend_from_slice(&1u64.to_le_bytes()); // min_sol_output; route-level min_return protects the leg

    let mut accounts = Vec::with_capacity(14);
    accounts.push(AccountMeta::new_readonly(swap_accounts.global.key(), false));
    accounts.push(AccountMeta::new(swap_accounts.fee_recipient.key(), false));
    accounts.push(AccountMeta::new_readonly(swap_accounts.mint.key(), false));
    accounts.push(AccountMeta::new(swap_accounts.bonding_curve.key(), false));
    accounts.push(AccountMeta::new(swap_accounts.associated_bonding_curve.key(), false));
    accounts.push(AccountMeta::new(swap_accounts.swap_source_token.key(), false));
    accounts.push(AccountMeta::new(swap_accounts.swap_authority_pubkey.key(), true));
    accounts.push(AccountMeta::new_readonly(swap_accounts.system_program.key(), false));
    accounts.push(AccountMeta::new(swap_accounts.creator_vault.key(), false));
    accounts.push(AccountMeta::new_readonly(swap_accounts.base_token_program.key(), false));
    accounts.push(AccountMeta::new_readonly(swap_accounts.event_authority.key(), false));
    accounts.push(AccountMeta::new_readonly(swap_accounts.dex_program_id.key(), false));
    accounts.push(AccountMeta::new_readonly(swap_accounts.fee_config.key(), false));
    accounts.push(AccountMeta::new_readonly(swap_accounts.fee_program.key(), false));

    let account_infos = [
        swap_accounts.global.to_account_info(),
        swap_accounts.fee_recipient.to_account_info(),
        swap_accounts.mint.to_account_info(),
        swap_accounts.bonding_curve.to_account_info(),
        swap_accounts.associated_bonding_curve.to_account_info(),
        swap_accounts.swap_source_token.to_account_info(),
        swap_accounts.swap_authority_pubkey.to_account_info(),
        swap_accounts.system_program.to_account_info(),
        swap_accounts.creator_vault.to_account_info(),
        swap_accounts.base_token_program.to_account_info(),
        swap_accounts.event_authority.to_account_info(),
        swap_accounts.dex_program_id.to_account_info(),
        swap_accounts.fee_config.to_account_info(),
        swap_accounts.fee_program.to_account_info(),
        swap_accounts.wsol_program.to_account_info(),
        swap_accounts.swap_destination_token.to_account_info(),
    ];

    let instruction =
        Instruction { program_id: swap_accounts.dex_program_id.key(), accounts, data };

    let dex_processor = &PumpfunSellProcessor;
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
        SELL_ACCOUNTS_LEN,
        proxy_swap,
        owner_seeds,
    )?;
    Ok(amount_out)
}

/*=================================== v2 (token settlement) ====================================*/

// Dex::PumpfunBuyV2 / Dex::PumpfunSellV2 ride the curve's v2 token-settlement
// instructions (buy_exact_quote_in_v2 / sell_v2): the quote side moves as
// TOKENS (USDC/USDT-quote curves, or wSOL wrapped), so there is no native-SOL
// bridging at all. Two caller-side obligations the wire pins:
//
//   - the quote-side user account (buy source / sell destination) is the
//     canonical ATA of the swap authority for the quote mint — pump validates
//     it (validate_user_quote_token_account); the Go side allocates
//     authority-owned canonical ATAs for v2 hops;
//   - the base-side user account only needs owner == authority (empirically
//     an SPL owner check at the debit, no canonicality), so custody and
//     per-tx temps keep working there.
//
// `user` still pays pump's on-first-use rents (volume accumulator + ATAs),
// so on the PDA the payer pre-funds a headroom and sweeps the residue back
// after the CPI — the same dance as the native buy, minus the close.
const ARGS_LEN_V2: usize = 24;

pub struct PumpfunV2Processor;

impl DexProcessor for PumpfunV2Processor {
    fn before_invoke(&self, account_infos: &[AccountInfo]) -> Result<u64> {
        let authority = account_infos.get(V2_AUTHORITY_IDX).unwrap();
        let before_authority_lamports = authority.lamports();
        if authority.key() == authority_pda::ID {
            if before_authority_lamports < PUMPFUN_V2_FUND_HEADROOM {
                let payer = account_infos.last().unwrap();
                transfer_sol(
                    payer.to_account_info(),
                    authority.to_account_info(),
                    PUMPFUN_V2_FUND_HEADROOM - before_authority_lamports,
                    None,
                )?;
            }
        }
        Ok(before_authority_lamports)
    }

    fn after_invoke(
        &self,
        account_infos: &[AccountInfo],
        _hop: usize,
        _owner_seeds: Option<&[&[&[u8]]]>,
        _before_authority_lamports: u64,
    ) -> Result<u64> {
        let authority = account_infos.get(V2_AUTHORITY_IDX).unwrap();
        if authority.key() == authority_pda::ID {
            let drain = authority.lamports().saturating_sub(MIN_SOL_ACCOUNT_RENT);
            require!(drain <= SOL_DIFF_LIMIT, ErrorCode::InvalidDiffLamports);
            if drain > 0 {
                let payer = account_infos.last().unwrap();
                transfer_sol(
                    authority.to_account_info(),
                    payer.to_account_info(),
                    drain,
                    Some(SA_AUTHORITY_SEED),
                )?;
                msg!("pumpfun v2: authority swept {} lamports back to payer", drain);
            }
        }
        Ok(0)
    }
}
// account_infos layout: CPI accounts in wire order, payer appended last
const V2_AUTHORITY_IDX: usize = 13; // `user` slot of both v2 instructions

pub struct PumpfunBuyV2Accounts<'info> {
    pub dex_program_id: &'info AccountInfo<'info>,
    pub swap_authority_pubkey: &'info AccountInfo<'info>,
    pub swap_source_token: InterfaceAccount<'info, TokenAccount>, // canonical ATA(authority, quote)
    pub swap_destination_token: InterfaceAccount<'info, TokenAccount>, // base out, owner == authority
    pub global: &'info AccountInfo<'info>,
    pub base_mint: &'info AccountInfo<'info>,
    pub quote_mint: &'info AccountInfo<'info>,
    pub base_token_program: &'info AccountInfo<'info>,
    pub quote_token_program: &'info AccountInfo<'info>,
    pub associated_token_program: &'info AccountInfo<'info>,
    pub fee_recipient: &'info AccountInfo<'info>,
    pub associated_quote_fee_recipient: &'info AccountInfo<'info>,
    pub buyback_fee_recipient: &'info AccountInfo<'info>,
    pub associated_quote_buyback_fee_recipient: &'info AccountInfo<'info>,
    pub bonding_curve: &'info AccountInfo<'info>,
    pub associated_base_bonding_curve: &'info AccountInfo<'info>,
    pub associated_quote_bonding_curve: &'info AccountInfo<'info>,
    pub creator_vault: &'info AccountInfo<'info>,
    pub associated_creator_vault: &'info AccountInfo<'info>,
    pub sharing_config: &'info AccountInfo<'info>,
    pub global_volume_accumulator: &'info AccountInfo<'info>,
    pub user_volume_accumulator: &'info AccountInfo<'info>,
    pub associated_user_volume_accumulator: &'info AccountInfo<'info>,
    pub fee_config: &'info AccountInfo<'info>,
    pub fee_program: &'info AccountInfo<'info>,
    pub system_program: &'info AccountInfo<'info>,
    pub event_authority: &'info AccountInfo<'info>,
    pub program: &'info AccountInfo<'info>,
}
const BUY_V2_ACCOUNTS_LEN: usize = 28;

impl<'info> PumpfunBuyV2Accounts<'info> {
    fn parse_accounts(accounts: &'info [AccountInfo<'info>], offset: usize) -> Result<Self> {
        let [
            dex_program_id, swap_authority_pubkey, swap_source_token, swap_destination_token,
            global, base_mint, quote_mint, base_token_program, quote_token_program,
            associated_token_program, fee_recipient, associated_quote_fee_recipient,
            buyback_fee_recipient, associated_quote_buyback_fee_recipient, bonding_curve,
            associated_base_bonding_curve, associated_quote_bonding_curve, creator_vault,
            associated_creator_vault, sharing_config, global_volume_accumulator,
            user_volume_accumulator, associated_user_volume_accumulator, fee_config,
            fee_program, system_program, event_authority, program,
        ]: &[AccountInfo<'info>; BUY_V2_ACCOUNTS_LEN] =
            array_ref![accounts, offset, BUY_V2_ACCOUNTS_LEN];

        Ok(Self {
            dex_program_id,
            swap_authority_pubkey,
            swap_source_token: InterfaceAccount::try_from(swap_source_token)?,
            swap_destination_token: InterfaceAccount::try_from(swap_destination_token)?,
            global, base_mint, quote_mint, base_token_program, quote_token_program,
            associated_token_program, fee_recipient, associated_quote_fee_recipient,
            buyback_fee_recipient, associated_quote_buyback_fee_recipient, bonding_curve,
            associated_base_bonding_curve, associated_quote_bonding_curve, creator_vault,
            associated_creator_vault, sharing_config, global_volume_accumulator,
            user_volume_accumulator, associated_user_volume_accumulator, fee_config,
            fee_program, system_program, event_authority, program,
        })
    }
}

#[allow(clippy::too_many_arguments)]
pub fn buy_v2<'a>(
    remaining_accounts: &'a [AccountInfo<'a>],
    amount_in: u64,
    offset: &mut usize,
    hop_accounts: &mut HopAccounts,
    hop: usize,
    proxy_swap: bool,
    owner_seeds: Option<&[&[&[u8]]]>,
    payer: Option<&AccountInfo<'a>>,
) -> Result<u64> {
    msg!("Dex::PumpfunBuyV2 amount_in: {}, offset: {}", amount_in, offset);
    require!(
        remaining_accounts.len() >= *offset + BUY_V2_ACCOUNTS_LEN,
        ErrorCode::InvalidAccountsLength
    );
    let mut swap_accounts = PumpfunBuyV2Accounts::parse_accounts(remaining_accounts, *offset)?;
    if swap_accounts.dex_program_id.key != &pumpfun_program::id() {
        return Err(ErrorCode::InvalidProgramId.into());
    }
    swap_accounts.bonding_curve.key().log();

    before_check(
        swap_accounts.swap_authority_pubkey,
        &swap_accounts.swap_source_token,
        swap_accounts.swap_destination_token.key(),
        hop_accounts,
        hop,
        proxy_swap,
        owner_seeds,
    )?;

    let mut data = Vec::with_capacity(ARGS_LEN_V2);
    data.extend_from_slice(PUMPFUN_BUY_EXACT_QUOTE_IN_V2_SELECTOR);
    data.extend_from_slice(&amount_in.to_le_bytes()); // spendable_quote_in (fees inside)
    data.extend_from_slice(&1u64.to_le_bytes()); // min_tokens_out; route-level min_return protects the leg

    let a = &swap_accounts;
    let accounts = vec![
        AccountMeta::new_readonly(a.global.key(), false),
        AccountMeta::new_readonly(a.base_mint.key(), false),
        AccountMeta::new_readonly(a.quote_mint.key(), false),
        AccountMeta::new_readonly(a.base_token_program.key(), false),
        AccountMeta::new_readonly(a.quote_token_program.key(), false),
        AccountMeta::new_readonly(a.associated_token_program.key(), false),
        AccountMeta::new(a.fee_recipient.key(), false),
        AccountMeta::new(a.associated_quote_fee_recipient.key(), false),
        AccountMeta::new(a.buyback_fee_recipient.key(), false),
        AccountMeta::new(a.associated_quote_buyback_fee_recipient.key(), false),
        AccountMeta::new(a.bonding_curve.key(), false),
        AccountMeta::new(a.associated_base_bonding_curve.key(), false),
        AccountMeta::new(a.associated_quote_bonding_curve.key(), false),
        AccountMeta::new(a.swap_authority_pubkey.key(), true),
        AccountMeta::new(a.swap_destination_token.key(), false),
        AccountMeta::new(a.swap_source_token.key(), false),
        AccountMeta::new(a.creator_vault.key(), false),
        AccountMeta::new(a.associated_creator_vault.key(), false),
        AccountMeta::new_readonly(a.sharing_config.key(), false),
        AccountMeta::new_readonly(a.global_volume_accumulator.key(), false),
        AccountMeta::new(a.user_volume_accumulator.key(), false),
        AccountMeta::new(a.associated_user_volume_accumulator.key(), false),
        AccountMeta::new_readonly(a.fee_config.key(), false),
        AccountMeta::new_readonly(a.fee_program.key(), false),
        AccountMeta::new_readonly(a.system_program.key(), false),
        AccountMeta::new_readonly(a.event_authority.key(), false),
        AccountMeta::new_readonly(a.program.key(), false),
    ];

    let account_infos = [
        a.global.to_account_info(),
        a.base_mint.to_account_info(),
        a.quote_mint.to_account_info(),
        a.base_token_program.to_account_info(),
        a.quote_token_program.to_account_info(),
        a.associated_token_program.to_account_info(),
        a.fee_recipient.to_account_info(),
        a.associated_quote_fee_recipient.to_account_info(),
        a.buyback_fee_recipient.to_account_info(),
        a.associated_quote_buyback_fee_recipient.to_account_info(),
        a.bonding_curve.to_account_info(),
        a.associated_base_bonding_curve.to_account_info(),
        a.associated_quote_bonding_curve.to_account_info(),
        a.swap_authority_pubkey.to_account_info(),
        a.swap_destination_token.to_account_info(),
        a.swap_source_token.to_account_info(),
        a.creator_vault.to_account_info(),
        a.associated_creator_vault.to_account_info(),
        a.sharing_config.to_account_info(),
        a.global_volume_accumulator.to_account_info(),
        a.user_volume_accumulator.to_account_info(),
        a.associated_user_volume_accumulator.to_account_info(),
        a.fee_config.to_account_info(),
        a.fee_program.to_account_info(),
        a.system_program.to_account_info(),
        a.event_authority.to_account_info(),
        a.program.to_account_info(),
        payer.unwrap().to_account_info(),
    ];

    let instruction =
        Instruction { program_id: swap_accounts.dex_program_id.key(), accounts, data };

    invoke_process(
        amount_in,
        &PumpfunV2Processor,
        &account_infos,
        &mut swap_accounts.swap_source_token,
        &mut swap_accounts.swap_destination_token,
        hop_accounts,
        instruction,
        hop,
        offset,
        BUY_V2_ACCOUNTS_LEN,
        proxy_swap,
        owner_seeds,
    )
}

pub struct PumpfunSellV2Accounts<'info> {
    pub dex_program_id: &'info AccountInfo<'info>,
    pub swap_authority_pubkey: &'info AccountInfo<'info>,
    pub swap_source_token: InterfaceAccount<'info, TokenAccount>, // base in, owner == authority
    pub swap_destination_token: InterfaceAccount<'info, TokenAccount>, // canonical ATA(authority, quote)
    pub global: &'info AccountInfo<'info>,
    pub base_mint: &'info AccountInfo<'info>,
    pub quote_mint: &'info AccountInfo<'info>,
    pub base_token_program: &'info AccountInfo<'info>,
    pub quote_token_program: &'info AccountInfo<'info>,
    pub associated_token_program: &'info AccountInfo<'info>,
    pub fee_recipient: &'info AccountInfo<'info>,
    pub associated_quote_fee_recipient: &'info AccountInfo<'info>,
    pub buyback_fee_recipient: &'info AccountInfo<'info>,
    pub associated_quote_buyback_fee_recipient: &'info AccountInfo<'info>,
    pub bonding_curve: &'info AccountInfo<'info>,
    pub associated_base_bonding_curve: &'info AccountInfo<'info>,
    pub associated_quote_bonding_curve: &'info AccountInfo<'info>,
    pub creator_vault: &'info AccountInfo<'info>,
    pub associated_creator_vault: &'info AccountInfo<'info>,
    pub sharing_config: &'info AccountInfo<'info>,
    pub user_volume_accumulator: &'info AccountInfo<'info>,
    pub associated_user_volume_accumulator: &'info AccountInfo<'info>,
    pub fee_config: &'info AccountInfo<'info>,
    pub fee_program: &'info AccountInfo<'info>,
    pub system_program: &'info AccountInfo<'info>,
    pub event_authority: &'info AccountInfo<'info>,
    pub program: &'info AccountInfo<'info>,
}
const SELL_V2_ACCOUNTS_LEN: usize = 27;

impl<'info> PumpfunSellV2Accounts<'info> {
    fn parse_accounts(accounts: &'info [AccountInfo<'info>], offset: usize) -> Result<Self> {
        let [
            dex_program_id, swap_authority_pubkey, swap_source_token, swap_destination_token,
            global, base_mint, quote_mint, base_token_program, quote_token_program,
            associated_token_program, fee_recipient, associated_quote_fee_recipient,
            buyback_fee_recipient, associated_quote_buyback_fee_recipient, bonding_curve,
            associated_base_bonding_curve, associated_quote_bonding_curve, creator_vault,
            associated_creator_vault, sharing_config, user_volume_accumulator,
            associated_user_volume_accumulator, fee_config, fee_program, system_program,
            event_authority, program,
        ]: &[AccountInfo<'info>; SELL_V2_ACCOUNTS_LEN] =
            array_ref![accounts, offset, SELL_V2_ACCOUNTS_LEN];

        Ok(Self {
            dex_program_id,
            swap_authority_pubkey,
            swap_source_token: InterfaceAccount::try_from(swap_source_token)?,
            swap_destination_token: InterfaceAccount::try_from(swap_destination_token)?,
            global, base_mint, quote_mint, base_token_program, quote_token_program,
            associated_token_program, fee_recipient, associated_quote_fee_recipient,
            buyback_fee_recipient, associated_quote_buyback_fee_recipient, bonding_curve,
            associated_base_bonding_curve, associated_quote_bonding_curve, creator_vault,
            associated_creator_vault, sharing_config, user_volume_accumulator,
            associated_user_volume_accumulator, fee_config, fee_program, system_program,
            event_authority, program,
        })
    }
}

#[allow(clippy::too_many_arguments)]
pub fn sell_v2<'a>(
    remaining_accounts: &'a [AccountInfo<'a>],
    amount_in: u64,
    offset: &mut usize,
    hop_accounts: &mut HopAccounts,
    hop: usize,
    proxy_swap: bool,
    owner_seeds: Option<&[&[&[u8]]]>,
    payer: Option<&AccountInfo<'a>>,
) -> Result<u64> {
    msg!("Dex::PumpfunSellV2 amount_in: {}, offset: {}", amount_in, offset);
    require!(
        remaining_accounts.len() >= *offset + SELL_V2_ACCOUNTS_LEN,
        ErrorCode::InvalidAccountsLength
    );
    let mut swap_accounts = PumpfunSellV2Accounts::parse_accounts(remaining_accounts, *offset)?;
    if swap_accounts.dex_program_id.key != &pumpfun_program::id() {
        return Err(ErrorCode::InvalidProgramId.into());
    }
    swap_accounts.bonding_curve.key().log();

    before_check(
        swap_accounts.swap_authority_pubkey,
        &swap_accounts.swap_source_token,
        swap_accounts.swap_destination_token.key(),
        hop_accounts,
        hop,
        proxy_swap,
        owner_seeds,
    )?;

    let mut data = Vec::with_capacity(ARGS_LEN_V2);
    data.extend_from_slice(PUMPFUN_SELL_V2_SELECTOR);
    data.extend_from_slice(&amount_in.to_le_bytes()); // base amount in
    data.extend_from_slice(&1u64.to_le_bytes()); // min quote out; route-level min_return protects the leg

    let a = &swap_accounts;
    let accounts = vec![
        AccountMeta::new_readonly(a.global.key(), false),
        AccountMeta::new_readonly(a.base_mint.key(), false),
        AccountMeta::new_readonly(a.quote_mint.key(), false),
        AccountMeta::new_readonly(a.base_token_program.key(), false),
        AccountMeta::new_readonly(a.quote_token_program.key(), false),
        AccountMeta::new_readonly(a.associated_token_program.key(), false),
        AccountMeta::new(a.fee_recipient.key(), false),
        AccountMeta::new(a.associated_quote_fee_recipient.key(), false),
        AccountMeta::new(a.buyback_fee_recipient.key(), false),
        AccountMeta::new(a.associated_quote_buyback_fee_recipient.key(), false),
        AccountMeta::new(a.bonding_curve.key(), false),
        AccountMeta::new(a.associated_base_bonding_curve.key(), false),
        AccountMeta::new(a.associated_quote_bonding_curve.key(), false),
        AccountMeta::new(a.swap_authority_pubkey.key(), true),
        AccountMeta::new(a.swap_source_token.key(), false),
        AccountMeta::new(a.swap_destination_token.key(), false),
        AccountMeta::new(a.creator_vault.key(), false),
        AccountMeta::new(a.associated_creator_vault.key(), false),
        AccountMeta::new_readonly(a.sharing_config.key(), false),
        AccountMeta::new(a.user_volume_accumulator.key(), false),
        AccountMeta::new(a.associated_user_volume_accumulator.key(), false),
        AccountMeta::new_readonly(a.fee_config.key(), false),
        AccountMeta::new_readonly(a.fee_program.key(), false),
        AccountMeta::new_readonly(a.system_program.key(), false),
        AccountMeta::new_readonly(a.event_authority.key(), false),
        AccountMeta::new_readonly(a.program.key(), false),
    ];

    let account_infos = [
        a.global.to_account_info(),
        a.base_mint.to_account_info(),
        a.quote_mint.to_account_info(),
        a.base_token_program.to_account_info(),
        a.quote_token_program.to_account_info(),
        a.associated_token_program.to_account_info(),
        a.fee_recipient.to_account_info(),
        a.associated_quote_fee_recipient.to_account_info(),
        a.buyback_fee_recipient.to_account_info(),
        a.associated_quote_buyback_fee_recipient.to_account_info(),
        a.bonding_curve.to_account_info(),
        a.associated_base_bonding_curve.to_account_info(),
        a.associated_quote_bonding_curve.to_account_info(),
        a.swap_authority_pubkey.to_account_info(),
        a.swap_source_token.to_account_info(),
        a.swap_destination_token.to_account_info(),
        a.creator_vault.to_account_info(),
        a.associated_creator_vault.to_account_info(),
        a.sharing_config.to_account_info(),
        a.user_volume_accumulator.to_account_info(),
        a.associated_user_volume_accumulator.to_account_info(),
        a.fee_config.to_account_info(),
        a.fee_program.to_account_info(),
        a.system_program.to_account_info(),
        a.event_authority.to_account_info(),
        a.program.to_account_info(),
        payer.unwrap().to_account_info(),
    ];

    let instruction =
        Instruction { program_id: swap_accounts.dex_program_id.key(), accounts, data };

    invoke_process(
        amount_in,
        &PumpfunV2Processor,
        &account_infos,
        &mut swap_accounts.swap_source_token,
        &mut swap_accounts.swap_destination_token,
        hop_accounts,
        instruction,
        hop,
        offset,
        SELL_V2_ACCOUNTS_LEN,
        proxy_swap,
        owner_seeds,
    )
}
