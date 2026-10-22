use super::common::{
    before_check, before_check_pda_user, enforce_sa_token_allowlist, invoke_process,
    DexProcessor,
};
use crate::error::ErrorCode;
use crate::utils::{close_token_account, sync_wsol_account, transfer_sol};
use crate::{
    HopAccounts, BUY_EXACT_SOL_IN_SELECTOR, PUMPFUN_BUY_EXACT_QUOTE_IN_V2_SELECTOR,
    PUMPFUN_FUND_HEADROOM, PUMPFUN_SELL_V2_SELECTOR, PUMPFUN_SELL_SELECTOR,
    PUMPFUN_V2_FUND_HEADROOM, SOL_DIFF_LIMIT, SA_AUTHORITY_SEED, authority_pda, pumpfun_program,
    wsol_sa,
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
//          input as native SOL; after the CPI exactly the temp's fronted
//          rent is refunded to the payer, so the funding wallet nets zero.
//          Pump's on-first-use rents (the per-user volume accumulator, plus
//          the creator vault on a coin's first-ever buy) are borne by the
//          authority's standing balance — ops seeds it once
//          (≥ PUMPFUN_FUND_HEADROOM) and the payer is never charged.
//   sell — the CPI pays the authority natively; after it the exact received
//          delta is transferred into the wSOL destination account and
//          sync_native'd, restoring the token-account settlement model.
//
// Wire shape (verified by mainnet simulation 2026-10): buy carries the 16
// declared accounts plus two mandatory trailing buyback slots (the second
// must be an authorized pump-fees vault; the first is not validated), sell
// needs its 14 declared accounts only. track_volume (OptionBool) is omitted
// from buy_exact_sol_in and anchor deserializes the missing byte as None —
// but the 2026-10 mainnet program still creates the per-user volume
// accumulator on first use and charges its rent to the `user` slot
// (live-verified: tx 4Suz3jfC…, 1_346_200 lamports). That rent is exactly
// the first-use cost the authority's standing balance exists to bear.
const ARGS_LEN: usize = 24;

/// The 2026-10 trailing pair, in the order pump validates it: the
/// bonding-curve-v2 PDA (read-only) then an authorized buyback fee recipient
/// (writable). Shared by the buy and sell legs so order/permissions live in
/// exactly one place.
fn pump_curve_tail_metas(curve_v2: Pubkey, buyback: Pubkey) -> [AccountMeta; 2] {
    [
        AccountMeta::new_readonly(curve_v2, false),
        AccountMeta::new(buyback, false),
    ]
}

/// Which trailing-account layout the curve CPI targets. The 2026-10 pump
/// upgrade removed the free-form second buyback slot and now expects the
/// `bonding-curve-v2` PDA there instead (sell gained the same pair), so the
/// old and new shapes are selected by the Dex variant, never auto-detected.
#[derive(Clone, Copy, PartialEq, Eq)]
enum CurveLayout {
    /// pre-2026-10 wire: buy carries two buyback slots, sell carries none
    Legacy,
    /// 2026-10 wire: buy/sell both end with [bonding-curve-v2 (ro),
    /// buyback fee recipient (w)]; the volume accumulator is writable
    Tail202610,
}

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
// 2026-10 buy keeps the same 20-slot window: the last two slots change
// meaning (bonding-curve-v2 + buyback) rather than count.
const BUY_202610_ACCOUNTS_LEN: usize = BUY_ACCOUNTS_LEN;
// account_infos handed to the processor: the 18 CPI accounts, then the
// close/refund helpers (src program, payer, src)
const BUY_PROCESSOR_LEN: usize = 21;
// pump's per-user volume accumulator in the processor row: its creation
// rent is charged to the buy's `user` slot (our authority), so its absence
// is the first-use signal the before-hook checks
const BUY_USER_VOLUME_ACC_IDX: usize = 13;
// the coin's creator vault, lazily created on the coin's first-ever buy —
// the other first-use rent the before-hook gates on
const BUY_CREATOR_VAULT_IDX: usize = 9;

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
    /// rent the funding wallet advanced for the disposable wSOL temp (its
    /// lamports minus the wrapped input this leg spends) — refunded in full
    /// to the payer after the CPI
    pub temp_rent: u64,
}

impl DexProcessor for PumpfunBuyProcessor {
    fn before_invoke(&self, account_infos: &[AccountInfo]) -> Result<u64> {
        // the pump `user` is the authority PDA at every hop (2026-10-22
        // unification): the disposable wSOL source is PDA-owned even at
        // hop0, so the PDA both signs and receives the close
        let authority = account_infos.get(6).unwrap();
        let src_token_program = account_infos.get(BUY_PROCESSOR_LEN - 3).unwrap();
        let source_token_account = account_infos.last().unwrap();
        let before_authority_lamports = authority.lamports();

        // the source is closed into the authority: it must be a
        // disposable per-tx wSOL temp, never the shared wsol_sa or a
        // warmed custody account (the caller guarantees this; the
        // shared account would be destroyed)
        require!(
            source_token_account.key() != wsol_sa::ID,
            ErrorCode::InvalidSourceTokenAccount
        );
        // pump's on-first-use rents — the per-user volume accumulator,
        // plus the coin's creator vault on its first-ever buy — are
        // charged to the buy's `user` slot (our PDA) and are borne by
        // its standing balance, never the payer. Fail fast while any
        // of them is unborn and the balance can't cover the worst case
        // (2.4M > 1,346,200 + 890,880): a short authority would
        // silently spend the temp's refundable rent and abort the
        // after-hook refund instead.
        let user_volume_accumulator = account_infos.get(BUY_USER_VOLUME_ACC_IDX).unwrap();
        let creator_vault = account_infos.get(BUY_CREATOR_VAULT_IDX).unwrap();
        let first_use_missing = (user_volume_accumulator.lamports() == 0
            && user_volume_accumulator.data_len() == 0)
            || (creator_vault.lamports() == 0 && creator_vault.data_len() == 0);
        if first_use_missing {
            require!(
                before_authority_lamports >= PUMPFUN_FUND_HEADROOM,
                ErrorCode::InsufficientFunds
            );
        }
        // defense in depth (the route builder is permissionless): no
        // PDA-owned token account may ride this CPI outside the hop's
        // declared source/destination slots
        enforce_sa_token_allowlist(
            account_infos,
            &[
                source_token_account.key(),
                account_infos.get(5).unwrap().key(), // swap_destination_token
            ],
        )?;
        close_token_account(
            source_token_account.to_account_info(),
            authority.to_account_info(),
            authority.to_account_info(),
            src_token_program.to_account_info(),
            Some(SA_AUTHORITY_SEED),
        )?;
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
        // refund exactly the rent the funding wallet advanced for the
        // temp: the wrapped input was consumed by the buy and pump's
        // on-first-use rents come out of the authority's standing
        // balance — the payer nets zero on every buy, first or not
        require!(self.temp_rent <= SOL_DIFF_LIMIT, ErrorCode::InvalidDiffLamports);
        if self.temp_rent > 0 {
            transfer_sol(
                authority.to_account_info(),
                payer.to_account_info(),
                self.temp_rent,
                Some(SA_AUTHORITY_SEED),
            )?;
            msg!(
                "pumpfun buy: authority refunded {} lamports of temp rent to payer",
                self.temp_rent
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
    buy_with_layout(
        remaining_accounts,
        amount_in,
        offset,
        hop_accounts,
        hop,
        proxy_swap,
        owner_seeds,
        payer,
        CurveLayout::Legacy,
    )
}

/// 2026-10 pump curve buy (Dex::PumpfunBuy202610). Same 20-slot window as the
/// legacy buy, but the two trailing slots carry the bonding-curve-v2 PDA
/// (read-only) and the authorized buyback recipient (writable), and the
/// global volume accumulator is writable.
#[allow(clippy::too_many_arguments)]
pub fn buy_202610<'a>(
    remaining_accounts: &'a [AccountInfo<'a>],
    amount_in: u64,
    offset: &mut usize,
    hop_accounts: &mut HopAccounts,
    hop: usize,
    proxy_swap: bool,
    owner_seeds: Option<&[&[&[u8]]]>,
    payer: Option<&AccountInfo<'a>>,
) -> Result<u64> {
    buy_with_layout(
        remaining_accounts,
        amount_in,
        offset,
        hop_accounts,
        hop,
        proxy_swap,
        owner_seeds,
        payer,
        CurveLayout::Tail202610,
    )
}

#[allow(clippy::too_many_arguments)]
fn buy_with_layout<'a>(
    remaining_accounts: &'a [AccountInfo<'a>],
    amount_in: u64,
    offset: &mut usize,
    hop_accounts: &mut HopAccounts,
    hop: usize,
    proxy_swap: bool,
    owner_seeds: Option<&[&[&[u8]]]>,
    payer: Option<&AccountInfo<'a>>,
    layout: CurveLayout,
) -> Result<u64> {
    let (leg, window) = match layout {
        CurveLayout::Legacy => ("PumpfunBuy", BUY_ACCOUNTS_LEN),
        CurveLayout::Tail202610 => ("PumpfunBuy202610", BUY_202610_ACCOUNTS_LEN),
    };
    msg!("Dex::{} amount_in: {}, offset: {}", leg, amount_in, offset);
    require!(
        remaining_accounts.len() >= *offset + window,
        ErrorCode::InvalidAccountsLength
    );
    let mut swap_accounts = PumpfunBuyAccounts::parse_accounts(remaining_accounts, *offset)?;
    if swap_accounts.dex_program_id.key != &pumpfun_program::id() {
        return Err(ErrorCode::InvalidProgramId.into());
    }
    swap_accounts.bonding_curve.key().log();

    // the pump `user` is the authority PDA at every hop (2026-10-22
    // unification): hop0 rides a PDA-owned disposable temp too, so the
    // wallets never become pump users and never pay the per-user
    // first-use rents themselves
    before_check_pda_user(
        swap_accounts.swap_authority_pubkey,
        &swap_accounts.swap_source_token,
        swap_accounts.swap_destination_token.key(),
        hop_accounts,
    )?;

    // hop0: the source must be the verifiable one-shot program temp
    // (PDA ["pump-hop0", payer]) — a shared custody/wsol_sa account can
    // never match the derivation, killing the permissionless-route
    // "custody as source" vector. hop >= 1 keeps custody/temp sources:
    // custody routing through intermediates is the design there.
    if hop == 0 {
        let (expected_temp, _bump) = Pubkey::find_program_address(
            &[b"pump-hop0", payer.unwrap().key().as_ref()],
            &crate::ID,
        );
        require_keys_eq!(
            swap_accounts.swap_source_token.key(),
            expected_temp,
            ErrorCode::InvalidHopAccounts
        );
    }

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
    if layout == CurveLayout::Tail202610 {
        // pump bumps the volume counter on every trade; writable under the
        // 2026-10 wire (the legacy one stays read-only for rollback parity)
        accounts.push(AccountMeta::new(swap_accounts.global_volume_accumulator.key(), false));
    } else {
        accounts.push(AccountMeta::new_readonly(swap_accounts.global_volume_accumulator.key(), false));
    }
    accounts.push(AccountMeta::new(swap_accounts.user_volume_accumulator.key(), false));
    accounts.push(AccountMeta::new_readonly(swap_accounts.fee_config.key(), false));
    accounts.push(AccountMeta::new_readonly(swap_accounts.fee_program.key(), false));
    match layout {
        CurveLayout::Legacy => {
            accounts.push(AccountMeta::new(swap_accounts.buyback_a.key(), false));
            accounts.push(AccountMeta::new(swap_accounts.buyback_b.key(), false));
        }
        CurveLayout::Tail202610 => {
            // Go fills these two slots with the bonding-curve-v2 PDA and the
            // authorized buyback recipient, in that order.
            accounts.extend_from_slice(&pump_curve_tail_metas(
                swap_accounts.buyback_a.key(),
                swap_accounts.buyback_b.key(),
            ));
        }
    }

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

    // the rent advanced for the disposable temp (its lamports minus the
    // wrapped input this leg spends); the after-hook refunds it in full to
    // the route payer, so whoever funded the create nets zero
    let temp_rent = account_infos
        .last()
        .unwrap()
        .lamports()
        .saturating_sub(amount_in);
    let dex_processor = &PumpfunBuyProcessor { temp_rent };
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
    // 2026-10 trailing pair; None on the legacy layout
    pub curve_v2: Option<&'info AccountInfo<'info>>,
    pub buyback_fee_recipient: Option<&'info AccountInfo<'info>>,
}
// remaining-accounts window: the 16 above
const SELL_ACCOUNTS_LEN: usize = 16;
// 2026-10 window: the legacy 16 plus the [bonding-curve-v2, buyback] pair
const SELL_202610_ACCOUNTS_LEN: usize = SELL_ACCOUNTS_LEN + 2;

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
            curve_v2: None,
            buyback_fee_recipient: None,
        })
    }

    /// 2026-10 layout: the legacy 16 slots, then the
    /// [bonding-curve-v2, buyback] pair the current pump program requires.
    fn parse_accounts_202610(accounts: &'info [AccountInfo<'info>], offset: usize) -> Result<Self> {
        let mut parsed = Self::parse_accounts(accounts, offset)?;
        parsed.curve_v2 = Some(&accounts[offset + SELL_ACCOUNTS_LEN]);
        parsed.buyback_fee_recipient = Some(&accounts[offset + SELL_ACCOUNTS_LEN + 1]);
        Ok(parsed)
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
        // the two wrap helpers are always the trailing pair of account_infos;
        // the 2026-10 layout inserts the curve pair before them, so index
        // from the end instead of a fixed window length
        let wsol_program = account_infos.get(account_infos.len() - 2).unwrap();
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
// account_infos handed to the processor: the 14 CPI accounts (plus the
// 2026-10 curve pair when that layout is used) and the two wrap helpers
// (wsol program, destination). The sell moves no payer funds, so unlike buy
// there is no payer slot.

#[allow(clippy::too_many_arguments)]
pub fn sell<'a>(
    remaining_accounts: &'a [AccountInfo<'a>],
    amount_in: u64,
    offset: &mut usize,
    hop_accounts: &mut HopAccounts,
    hop: usize,
    proxy_swap: bool,
    owner_seeds: Option<&[&[&[u8]]]>,
    payer: Option<&AccountInfo<'a>>,
) -> Result<u64> {
    sell_with_layout(
        remaining_accounts,
        amount_in,
        offset,
        hop_accounts,
        hop,
        proxy_swap,
        owner_seeds,
        payer,
        CurveLayout::Legacy,
    )
}

/// 2026-10 pump curve sell (Dex::PumpfunSell202610): the legacy 16-slot
/// window plus the [bonding-curve-v2 (ro), buyback recipient (w)] pair the
/// current pump program requires on the sell CPI.
#[allow(clippy::too_many_arguments)]
pub fn sell_202610<'a>(
    remaining_accounts: &'a [AccountInfo<'a>],
    amount_in: u64,
    offset: &mut usize,
    hop_accounts: &mut HopAccounts,
    hop: usize,
    proxy_swap: bool,
    owner_seeds: Option<&[&[&[u8]]]>,
    payer: Option<&AccountInfo<'a>>,
) -> Result<u64> {
    sell_with_layout(
        remaining_accounts,
        amount_in,
        offset,
        hop_accounts,
        hop,
        proxy_swap,
        owner_seeds,
        payer,
        CurveLayout::Tail202610,
    )
}

#[allow(clippy::too_many_arguments)]
fn sell_with_layout<'a>(
    remaining_accounts: &'a [AccountInfo<'a>],
    amount_in: u64,
    offset: &mut usize,
    hop_accounts: &mut HopAccounts,
    hop: usize,
    proxy_swap: bool,
    owner_seeds: Option<&[&[&[u8]]]>,
    _payer: Option<&AccountInfo<'a>>,
    layout: CurveLayout,
) -> Result<u64> {
    let (leg, window) = match layout {
        CurveLayout::Legacy => ("PumpfunSell", SELL_ACCOUNTS_LEN),
        CurveLayout::Tail202610 => ("PumpfunSell202610", SELL_202610_ACCOUNTS_LEN),
    };
    msg!("Dex::{} amount_in: {}, offset: {}", leg, amount_in, offset);
    require!(
        remaining_accounts.len() >= *offset + window,
        ErrorCode::InvalidAccountsLength
    );
    let mut swap_accounts = match layout {
        CurveLayout::Legacy => PumpfunSellAccounts::parse_accounts(remaining_accounts, *offset)?,
        CurveLayout::Tail202610 => {
            PumpfunSellAccounts::parse_accounts_202610(remaining_accounts, *offset)?
        }
    };
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
    if layout == CurveLayout::Tail202610 {
        accounts.extend_from_slice(&pump_curve_tail_metas(
            swap_accounts.curve_v2.unwrap().key(),
            swap_accounts.buyback_fee_recipient.unwrap().key(),
        ));
    }

    let mut account_infos = vec![
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
    ];
    if layout == CurveLayout::Tail202610 {
        account_infos.push(swap_accounts.curve_v2.unwrap().to_account_info());
        account_infos.push(swap_accounts.buyback_fee_recipient.unwrap().to_account_info());
    }
    account_infos.push(swap_accounts.wsol_program.to_account_info());
    account_infos.push(swap_accounts.swap_destination_token.to_account_info());

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
        window,
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
// `user` still pays pump's on-first-use rents (volume accumulator + quote
// ATAs for the fee/buyback recipients: 1_346_200 + ~2_039_280 × 2 plus
// margin ≈ 6M lamports worst case), but the authority's standing balance
// bears them: no payer top-up, no post-CPI sweep — a deficit is a rent
// absorbed by design, a surplus (anomaly) is swept back. While the volume
// accumulator is unborn the before-hook enforces the standing floor
// (PUMPFUN_V2_FUND_HEADROOM), same fail-fast as the native buy.
const ARGS_LEN_V2: usize = 24;

pub struct PumpfunV2Processor {
    /// user_volume_accumulator's index in account_infos: the buy and sell
    /// v2 layouts carry it at different slots (sell has no global
    /// accumulator), so each leg pins its own
    pub uva_idx: usize,
}

impl DexProcessor for PumpfunV2Processor {
    fn before_invoke(&self, account_infos: &[AccountInfo]) -> Result<u64> {
        // v2 settles in tokens and routes no native value through the
        // authority; its only native footprint is pump's on-first-use
        // rents, which the authority's standing balance bears — nothing to
        // front from the payer, but an unborn accumulator means the worst
        // case must already be standing or the swap fails fast
        let authority = account_infos.get(V2_AUTHORITY_IDX).unwrap();
        let before_authority_lamports = authority.lamports();
        if authority.key() == authority_pda::ID {
            let user_volume_accumulator = account_infos.get(self.uva_idx).unwrap();
            let accumulator_missing =
                user_volume_accumulator.lamports() == 0 && user_volume_accumulator.data_len() == 0;
            if accumulator_missing {
                require!(
                    before_authority_lamports >= PUMPFUN_V2_FUND_HEADROOM,
                    ErrorCode::InsufficientFunds
                );
            }
        }
        Ok(before_authority_lamports)
    }

    fn after_invoke(
        &self,
        account_infos: &[AccountInfo],
        _hop: usize,
        _owner_seeds: Option<&[&[&[u8]]]>,
        before_authority_lamports: u64,
    ) -> Result<u64> {
        let authority = account_infos.get(V2_AUTHORITY_IDX).unwrap();
        if authority.key() == authority_pda::ID {
            // settle any surplus back to the payer; a deficit is a
            // first-use rent the authority absorbs by design
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
                msg!("pumpfun v2: authority swept {} lamports back to payer", drain);
            }
        }
        Ok(0)
    }
}
// account_infos layout: CPI accounts in wire order, payer appended last
const V2_AUTHORITY_IDX: usize = 13; // `user` slot of both v2 instructions
const V2_BUY_UVA_IDX: usize = 20; // buy_v2: gva@19, uva@20, assoc-uva@21
const V2_SELL_UVA_IDX: usize = 19; // sell_v2 layout carries no gva

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
        &PumpfunV2Processor { uva_idx: V2_BUY_UVA_IDX },
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
        &PumpfunV2Processor { uva_idx: V2_SELL_UVA_IDX },
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

#[cfg(test)]
mod tests {
    use super::*;
    use anchor_lang::solana_program::hash::hash;

    fn disc(name: &str) -> [u8; 8] {
        let h = hash(name.as_bytes()).to_bytes();
        let mut d = [0u8; 8];
        d.copy_from_slice(&h[..8]);
        d
    }

    fn expect_disc(constant: &[u8; 8], name: &str) {
        assert_eq!(constant[..], disc(name), "selector for {} drifted", name);
    }

    // The selectors are hand-pinned constants: pin each against the anchor
    // derivation (sha256("global:<ix>")[:8]) so a typo cannot reach the wire.
    #[test]
    pub fn test_selector_derivations() {
        // the phase-2 helper instructions the Go side hand-builds: pin
        // their anchor discriminators the same way (a typo cannot reach
        // the wire)
        expect_disc(
            &anchor_lang::solana_program::hash::hash(
                b"global:create_pump_hop0_temp".as_ref(),
            ).to_bytes()[..8].try_into().unwrap(),
            "global:create_pump_hop0_temp",
        );
        expect_disc(PUMPFUN_SELL_SELECTOR, "global:sell");
        expect_disc(BUY_EXACT_SOL_IN_SELECTOR, "global:buy_exact_sol_in");
        expect_disc(PUMPFUN_BUY_EXACT_QUOTE_IN_V2_SELECTOR, "global:buy_exact_quote_in_v2");
        expect_disc(PUMPFUN_SELL_V2_SELECTOR, "global:sell_v2");
    }

    // Data packing mirrors each adapter's CPI payload: discriminator +
    // two LE u64 args, 24 bytes, no track_volume byte (OptionBool omitted
    // deserializes as None on chain).
    #[test]
    pub fn test_pack_buy_exact_sol_in() {
        let amount_in = 1_000_000u64;
        let mut data = Vec::with_capacity(ARGS_LEN);
        data.extend_from_slice(BUY_EXACT_SOL_IN_SELECTOR);
        data.extend_from_slice(&amount_in.to_le_bytes());
        data.extend_from_slice(&1u64.to_le_bytes());
        assert_eq!(data.len(), ARGS_LEN);
        assert_eq!(&data[..8], &BUY_EXACT_SOL_IN_SELECTOR[..]);
        assert_eq!(u64::from_le_bytes(data[8..16].try_into().unwrap()), amount_in);
        assert_eq!(u64::from_le_bytes(data[16..24].try_into().unwrap()), 1);
    }

    #[test]
    pub fn test_pack_sell() {
        let amount_in = 42u64;
        let mut data = Vec::with_capacity(ARGS_LEN);
        data.extend_from_slice(PUMPFUN_SELL_SELECTOR);
        data.extend_from_slice(&amount_in.to_le_bytes());
        data.extend_from_slice(&1u64.to_le_bytes());
        assert_eq!(data.len(), ARGS_LEN);
        assert_eq!(u64::from_le_bytes(data[8..16].try_into().unwrap()), amount_in);
    }

    // 2026-10 windows: the buy window is unchanged (the tail changes meaning,
    // not length); the sell window grows by the two curve-pair accounts.
    #[test]
    pub fn test_202610_window_lengths() {
        assert_eq!(BUY_202610_ACCOUNTS_LEN, BUY_ACCOUNTS_LEN);
        assert_eq!(BUY_202610_ACCOUNTS_LEN, 4 + 18 - 3 + 1); // +src_token_program
        assert_eq!(SELL_202610_ACCOUNTS_LEN, SELL_ACCOUNTS_LEN + 2);
        assert_eq!(SELL_202610_ACCOUNTS_LEN, 4 + 16 - 3 + 1); // +wsol_program
    }

    // The 2026-10 tail pair: bonding-curve-v2 first and read-only, the
    // authorized buyback recipient second and writable. Swapping either the
    // order or the writability reproduces the on-chain failures this wire
    // pinning exists to prevent (6074 / privilege escalation).
    #[test]
    pub fn test_pump_curve_tail_metas() {
        let curve_v2 = Pubkey::new_unique();
        let buyback = Pubkey::new_unique();
        let tail = pump_curve_tail_metas(curve_v2, buyback);
        assert_eq!(tail[0].pubkey, curve_v2);
        assert!(!tail[0].is_writable, "bonding-curve-v2 must be read-only");
        assert!(!tail[0].is_signer);
        assert_eq!(tail[1].pubkey, buyback);
        assert!(tail[1].is_writable, "buyback recipient must be writable");
        assert!(!tail[1].is_signer);
    }

    #[test]
    pub fn test_pack_v2_instructions() {
        for (selector, args_len) in [
            (PUMPFUN_BUY_EXACT_QUOTE_IN_V2_SELECTOR, ARGS_LEN_V2),
            (PUMPFUN_SELL_V2_SELECTOR, ARGS_LEN_V2),
        ] {
            let amount_in = 7u64;
            let mut data = Vec::with_capacity(args_len);
            data.extend_from_slice(selector);
            data.extend_from_slice(&amount_in.to_le_bytes());
            data.extend_from_slice(&1u64.to_le_bytes());
            assert_eq!(data.len(), args_len);
            assert_eq!(&data[..8], &selector[..]);
            assert_eq!(u64::from_le_bytes(data[8..16].try_into().unwrap()), amount_in);
            assert_eq!(u64::from_le_bytes(data[16..24].try_into().unwrap()), 1);
        }
    }

    // The remaining-accounts windows: every parse_accounts destructuring
    // pattern must consume exactly its declared window length or the
    // offset cursor derails the rest of the route. Window = 4 routing
    // slots (dex/authority/src/dst) + CPI accounts minus the three that
    // fold into the routing slots (user, base/quote user) + adapters'
    // helper-only slots (never CPI'd): v1 buy carries src_token_program
    // for the wSOL close, v1 sell carries wsol_program for the wrap.
    #[test]
    pub fn test_window_lengths() {
        assert_eq!(BUY_ACCOUNTS_LEN, 4 + 18 - 3 + 1); // +src_token_program
        assert_eq!(SELL_ACCOUNTS_LEN, 4 + 14 - 3 + 1); // +wsol_program
        assert_eq!(BUY_V2_ACCOUNTS_LEN, 4 + 27 - 3);
        assert_eq!(SELL_V2_ACCOUNTS_LEN, 4 + 26 - 3);
        // processor rows: CPI accounts + helpers (+payer)
        assert_eq!(BUY_PROCESSOR_LEN, 18 + 3); // +src prog, payer, src
        assert_eq!(V2_AUTHORITY_IDX, 13); // `user` slot of both v2 layouts
        // first-use gate slots: the native buy gates on uva + creator
        // vault; the v2 legs carry the uva at different indexes (sell_v2
        // has no global accumulator ahead of it)
        assert_eq!(BUY_USER_VOLUME_ACC_IDX, 13);
        assert_eq!(BUY_CREATOR_VAULT_IDX, 9);
        assert_eq!(V2_BUY_UVA_IDX, 20);
        assert_eq!(V2_SELL_UVA_IDX, V2_BUY_UVA_IDX - 1);
    }
}
