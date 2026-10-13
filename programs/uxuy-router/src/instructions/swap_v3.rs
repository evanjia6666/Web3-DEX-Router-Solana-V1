use super::{SwapArgs, common_swap_v3};
use crate::error::ErrorCode;
use crate::processor::*;
use crate::SA_AUTHORITY_SEED;

use crate::utils::{log_fee_accounts_info, log_rate_info_v3, log_rate_info_v3_enhanced};
use crate::wsol_program;
use anchor_lang::prelude::*;
use anchor_spl::{
    associated_token::AssociatedToken,
    token_interface::{Mint, TokenAccount, TokenInterface},
};

#[derive(Accounts)]
pub struct CommissionProxySwapAccountsV3<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,

    #[account(
        mut,
        token::mint = source_mint,
        token::authority = payer,
    )]
    pub source_token_account: Box<InterfaceAccount<'info, TokenAccount>>,

    #[account(
        mut,
        token::mint = destination_mint,
    )]
    pub destination_token_account: Box<InterfaceAccount<'info, TokenAccount>>,

    pub source_mint: Box<InterfaceAccount<'info, Mint>>,

    pub destination_mint: Box<InterfaceAccount<'info, Mint>>,

    /// CHECK: commission account
    #[account(mut)]
    pub commission_account: Option<AccountInfo<'info>>,

    /// CHECK: platform fee account
    #[account(mut)]
    pub platform_fee_account: Option<AccountInfo<'info>>,

    /// CHECK: sa_authority
    #[account(mut)]
    pub sa_authority: Option<UncheckedAccount<'info>>,

    #[account(mut)]
    pub source_token_sa: Option<UncheckedAccount<'info>>,

    #[account(mut)]
    pub destination_token_sa: Option<UncheckedAccount<'info>>,

    pub source_token_program: Option<Interface<'info, TokenInterface>>,
    pub destination_token_program: Option<Interface<'info, TokenInterface>>,
    pub associated_token_program: Option<Program<'info, AssociatedToken>>,
    pub system_program: Option<Program<'info, System>>,
}

pub fn swap_tob_handler<'a>(
    ctx: Context<'_, '_, 'a, 'a, CommissionProxySwapAccountsV3<'a>>,
    args: SwapArgs,
    commission_info: u32,
    trim_rate: Option<u8>,
    platform_fee_rate: Option<u16>,
) -> Result<()> {
    let commission_direction = commission_info >> 31 == 1;
    let acc_close_flag = ((commission_info & (1 << 30)) >> 30) == 1;
    let commission_rate = commission_info & ((1 << 30) - 1);
    log_rate_info_v3(
        commission_rate,
        platform_fee_rate,
        trim_rate,
        commission_direction,
        acc_close_flag,
    );

    let trim_account = if trim_rate.is_some() && trim_rate.unwrap() > 0 {
        Some(&ctx.remaining_accounts[ctx.remaining_accounts.len() - 1])
    } else {
        None
    };
    common_swap_v3(
        &SwapToBProcessor,
        &ctx.accounts.payer,
        &mut ctx.accounts.source_token_account,
        &mut ctx.accounts.destination_token_account,
        &ctx.accounts.source_mint,
        &ctx.accounts.destination_mint,
        &mut ctx.accounts.sa_authority,
        &mut ctx.accounts.source_token_sa,
        &mut ctx.accounts.destination_token_sa,
        &ctx.accounts.source_token_program,
        &ctx.accounts.destination_token_program,
        &ctx.accounts.associated_token_program,
        &ctx.accounts.system_program,
        ctx.remaining_accounts,
        args,
        commission_rate,
        commission_direction,
        &ctx.accounts.commission_account,
        platform_fee_rate,
        &ctx.accounts.platform_fee_account,
        trim_rate,
        None,
        trim_account,
        None,
        acc_close_flag,
    )?;
    Ok(())
}

pub fn swap_toc_handler<'a>(
    ctx: Context<'_, '_, 'a, 'a, CommissionProxySwapAccountsV3<'a>>,
    args: SwapArgs,
    commission_info: u32,
    platform_fee_rate: Option<u16>,
) -> Result<()> {
    let commission_direction = commission_info >> 31 == 1;
    let commission_rate = commission_info & ((1 << 30) - 1);
    log_rate_info_v3(commission_rate, platform_fee_rate, None, commission_direction, false);

    // Log fee accounts info
    log_fee_accounts_info(&ctx.accounts.commission_account, &ctx.accounts.platform_fee_account);

    common_swap_v3(
        &SwapToCProcessor,
        &ctx.accounts.payer,
        &mut ctx.accounts.source_token_account,
        &mut ctx.accounts.destination_token_account,
        &ctx.accounts.source_mint,
        &ctx.accounts.destination_mint,
        &mut ctx.accounts.sa_authority,
        &mut ctx.accounts.source_token_sa,
        &mut ctx.accounts.destination_token_sa,
        &ctx.accounts.source_token_program,
        &ctx.accounts.destination_token_program,
        &ctx.accounts.associated_token_program,
        &ctx.accounts.system_program,
        ctx.remaining_accounts,
        args,
        commission_rate,
        commission_direction,
        &ctx.accounts.commission_account,
        platform_fee_rate,
        &ctx.accounts.platform_fee_account,
        None,
        None,
        None,
        None,
        false,
    )?;
    Ok(())
}

/// Account structure for swap with optional specified receiver
#[derive(Accounts)]
pub struct CommissionProxySwapAccountsV3WithReceiver<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,

    #[account(
        mut,
        token::mint = source_mint,
        token::authority = payer,
    )]
    pub source_token_account: Box<InterfaceAccount<'info, TokenAccount>>,

    #[account(
        mut,
        token::mint = destination_mint,
    )]
    pub destination_token_account: Box<InterfaceAccount<'info, TokenAccount>>,

    pub source_mint: Box<InterfaceAccount<'info, Mint>>,
    pub destination_mint: Box<InterfaceAccount<'info, Mint>>,

    /// CHECK: commission account
    #[account(mut)]
    pub commission_account: Option<AccountInfo<'info>>,

    /// CHECK: platform fee account
    #[account(mut)]
    pub platform_fee_account: Option<AccountInfo<'info>>,

    /// CHECK: sa_authority
    #[account(mut)]
    pub sa_authority: Option<UncheckedAccount<'info>>,

    #[account(mut)]
    pub source_token_sa: Option<UncheckedAccount<'info>>,

    #[account(mut)]
    pub destination_token_sa: Option<UncheckedAccount<'info>>,

    pub source_token_program: Option<Interface<'info, TokenInterface>>,
    pub destination_token_program: Option<Interface<'info, TokenInterface>>,
    pub associated_token_program: Option<Program<'info, AssociatedToken>>,
    pub system_program: Option<Program<'info, System>>,

    /// Optional SOL receiver account
    /// - None: normal swap or SOL stays with payer
    /// - Some: SOL receiver when converting wSOL -> SOL
    #[account(mut)]
    pub sol_receiver: Option<AccountInfo<'info>>,
}

/// Validate sol_receiver based on acc_close_flag
fn validate_sol_receiver_and_flags<'info>(
    sol_receiver: &Option<AccountInfo<'info>>,
    acc_close_flag: bool,
    destination_mint: &Pubkey,
) -> Result<()> {
    match (sol_receiver, acc_close_flag) {
        (Some(receiver), true) => {
            // Valid case: has receiver and acc_close_flag is true
            // Must be system account
            require!(
                receiver.owner == &anchor_lang::system_program::ID,
                ErrorCode::SolReceiverMustBeSystemAccount
            );
            // Must be wSOL destination
            require!(
                *destination_mint == wsol_program::ID,
                ErrorCode::DestinationMustBeWsolForSolReceiver
            );
            Ok(())
        }
        (Some(_), false) => {
            // Invalid: has receiver but acc_close_flag is false
            Err(ErrorCode::SolReceiverRequiresAccCloseFlag.into())
        }
        (None, _) => {
            // Valid: no receiver, any acc_close_flag is fine
            Ok(())
        }
    }
}

/// ToB swap handler with optional specified receiver
/// Supports both normal swaps and wSOL -> SOL conversion with custom receiver
/// SPL Token CloseAccount instruction tag.
const SPL_TOKEN_CLOSE_ACCOUNT: u8 = 51;

pub fn swap_tob_specified_receiver_handler<'a>(
    ctx: Context<'_, '_, 'a, 'a, CommissionProxySwapAccountsV3WithReceiver<'a>>,
    args: SwapArgs,
    commission_info: u32,
    trim_rate: Option<u8>,
    platform_fee_rate: Option<u16>,
) -> Result<()> {
    // Parse commission info
    let commission_direction = commission_info >> 31 == 1;
    let acc_close_flag = ((commission_info & (1 << 30)) >> 30) == 1;
    let commission_rate = commission_info & ((1 << 30) - 1);

    // Validate sol_receiver and acc_close_flag combination
    validate_sol_receiver_and_flags(
        &ctx.accounts.sol_receiver,
        acc_close_flag,
        &ctx.accounts.destination_mint.key(),
    )?;

    log_rate_info_v3(
        commission_rate,
        platform_fee_rate,
        trim_rate,
        commission_direction,
        acc_close_flag,
    );

    let trim_account = if trim_rate.is_some() && trim_rate.unwrap() > 0 {
        Some(&ctx.remaining_accounts[ctx.remaining_accounts.len() - 1])
    } else {
        None
    };

    // Execute swap and get actual amount out
    let (_, actual_amount_out, _, _) = common_swap_v3(
        &SwapToBProcessor,
        &ctx.accounts.payer,
        &mut ctx.accounts.source_token_account,
        &mut ctx.accounts.destination_token_account,
        &ctx.accounts.source_mint,
        &ctx.accounts.destination_mint,
        &mut ctx.accounts.sa_authority,
        &mut ctx.accounts.source_token_sa,
        &mut ctx.accounts.destination_token_sa,
        &ctx.accounts.source_token_program,
        &ctx.accounts.destination_token_program,
        &ctx.accounts.associated_token_program,
        &ctx.accounts.system_program,
        ctx.remaining_accounts,
        args,
        commission_rate,
        commission_direction,
        &ctx.accounts.commission_account,
        platform_fee_rate,
        &ctx.accounts.platform_fee_account,
        trim_rate,
        None,
        trim_account,
        None,
        acc_close_flag,
    )?;

    // Deliver native SOL to the specified receiver with OKX-V2 semantics
    // (audited on the reference transaction): the receiver gets EXACTLY the
    // swap output; the destination temp account's rent returns to the payer.
    // The destination must be the router-custody temp account (authority PDA
    // owned, created via create_token_account_with_seed): close it to the
    // payer (rent + wrapped balance back), then transfer the output amount
    // from the payer — a tx signer — to the receiver.
    if let Some(sol_receiver) = &ctx.accounts.sol_receiver {
        if acc_close_flag && actual_amount_out > 0 {
            let destination = ctx.accounts.destination_token_account.to_account_info();
            let payer_info = ctx.accounts.payer.to_account_info();
            let authority_info = ctx
                .accounts
                .sa_authority
                .as_ref()
                .ok_or(ErrorCode::InvalidSaAuthority)?
                .to_account_info();

            // token CloseAccount (tag 51): [account(w), destination(payer, w),
            // authority(ro, PDA signs via invoke_signed)]
            let close_ix = anchor_lang::solana_program::instruction::Instruction {
                program_id: crate::token_program::ID,
                accounts: vec![
                    AccountMeta::new(destination.key(), false),
                    AccountMeta::new(payer_info.key(), false),
                    AccountMeta::new_readonly(authority_info.key(), false),
                ],
                data: vec![SPL_TOKEN_CLOSE_ACCOUNT],
            };
            anchor_lang::solana_program::program::invoke_signed(
                &close_ix,
                &[destination.clone(), payer_info.clone(), authority_info.clone()],
                SA_AUTHORITY_SEED,
            )?;

            // output: payer (signer) -> receiver
            let transfer_ix = anchor_lang::solana_program::system_instruction::transfer(
                &payer_info.key(),
                &sol_receiver.key(),
                actual_amount_out,
            );
            anchor_lang::solana_program::program::invoke(
                &transfer_ix,
                &[payer_info.clone(), sol_receiver.to_account_info()],
            )?;
        }
    }

    Ok(())
}

pub fn swap_tob_enhanced_handler<'a>(
    ctx: Context<'_, '_, 'a, 'a, CommissionProxySwapAccountsV3<'a>>,
    args: SwapArgs,
    commission_info: u32,
    trim_rate: u8,
    charge_rate: u16,
    platform_fee_rate: Option<u16>,
) -> Result<()> {
    let commission_direction = commission_info >> 31 == 1;
    let acc_close_flag = ((commission_info & (1 << 30)) >> 30) == 1;
    let commission_rate = commission_info & ((1 << 30) - 1);
    log_rate_info_v3_enhanced(
        commission_rate,
        platform_fee_rate,
        trim_rate,
        charge_rate,
        commission_direction,
        acc_close_flag,
    );
    require!(trim_rate > 0 && charge_rate > 0, ErrorCode::InvalidTrimRate);

    let trim_account = &ctx.remaining_accounts[ctx.remaining_accounts.len() - 2];
    let charge_account = &ctx.remaining_accounts[ctx.remaining_accounts.len() - 1];

    common_swap_v3(
        &SwapToBProcessor,
        &ctx.accounts.payer,
        &mut ctx.accounts.source_token_account,
        &mut ctx.accounts.destination_token_account,
        &ctx.accounts.source_mint,
        &ctx.accounts.destination_mint,
        &mut ctx.accounts.sa_authority,
        &mut ctx.accounts.source_token_sa,
        &mut ctx.accounts.destination_token_sa,
        &ctx.accounts.source_token_program,
        &ctx.accounts.destination_token_program,
        &ctx.accounts.associated_token_program,
        &ctx.accounts.system_program,
        ctx.remaining_accounts,
        args,
        commission_rate,
        commission_direction,
        &ctx.accounts.commission_account,
        platform_fee_rate,
        &ctx.accounts.platform_fee_account,
        Some(trim_rate),
        Some(charge_rate),
        Some(trim_account),
        Some(charge_account),
        acc_close_flag,
    )?;
    Ok(())
}
