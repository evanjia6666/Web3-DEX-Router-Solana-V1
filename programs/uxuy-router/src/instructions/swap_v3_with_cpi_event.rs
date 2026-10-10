use super::{SwapArgs, common_swap_v3};
use crate::cpi_event::SwapWithFeesCpiEvent;
use crate::processor::*;
use crate::utils::*;
use anchor_lang::prelude::*;
use anchor_spl::{
    associated_token::AssociatedToken,
    token_interface::{Mint, TokenAccount, TokenInterface},
};

#[event_cpi]
#[derive(Accounts)]
pub struct SwapAccountsV3WithCpiEvent<'info> {
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

pub fn swap_toc_with_cpi_event_handler<'a>(
    ctx: Context<'_, '_, 'a, 'a, SwapAccountsV3WithCpiEvent<'a>>,
    args: SwapArgs,
    commission_info: u32,
    order_id: u64,
    platform_fee_rate: Option<u16>,
) -> Result<()> {
    let amount_in = args.amount_in;
    let source_token_account_owner = ctx.accounts.source_token_account.owner;
    let destination_token_account_owner = ctx.accounts.destination_token_account.owner;

    let commission_direction = commission_info >> 31 == 1;
    let commission_rate = commission_info & ((1 << 30) - 1);
    log_rate_info_v3(commission_rate, platform_fee_rate, None, commission_direction, false);

    // Log fee accounts info
    log_fee_accounts_info(&ctx.accounts.commission_account, &ctx.accounts.platform_fee_account);

    let (source_token_change, destination_token_change, commission_amount, platform_fee_amount) =
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
            order_id,
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

    let key_or_default =
        |acc: &Option<AccountInfo<'a>>| acc.as_ref().map(|a| a.key()).unwrap_or(Pubkey::default());

    // Log swap result
    emit_cpi!(SwapWithFeesCpiEvent {
        order_id,
        source_mint: ctx.accounts.source_mint.key(),
        destination_mint: ctx.accounts.destination_mint.key(),
        source_token_account_owner,
        destination_token_account_owner,
        amount_in,
        source_token_change,
        destination_token_change,
        commission_direction,
        commission_rate,
        commission_amount,
        commission_account: key_or_default(&ctx.accounts.commission_account),
        platform_fee_rate: platform_fee_rate.unwrap_or(0) as u16,
        platform_fee_amount,
        platform_fee_account: key_or_default(&ctx.accounts.platform_fee_account),
        trim_rate: 0,
        trim_amount: 0,
        trim_account: Pubkey::default(),
    });
    Ok(())
}
