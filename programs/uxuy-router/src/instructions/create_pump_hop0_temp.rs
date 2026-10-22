use crate::error::ErrorCode;
use crate::authority_pda;
use anchor_lang::prelude::*;
use anchor_lang::solana_program::program::{invoke, invoke_signed};
use anchor_lang::solana_program::system_instruction;
use anchor_spl::token::TokenAccount;
use anchor_spl::token_interface::{self, Mint, TokenInterface};

// One-shot PDA temp for a pump hop0 buy: the address is derived from the
// route payer alone (["pump-hop0", payer]), so the swap-side verifiers can
// PROVE a hop0 pump source is exactly this temp — a shared custody/wsol_sa
// account can never match the derivation, killing the permissionless-route
// "custody as source" vector. Created and closed inside the same
// transaction (create_account fails deterministically on a same-slot
// address collision from the same payer; the losing tx retries cleanly).
#[derive(Accounts)]
pub struct CreatePumpHop0TempAccounts<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,

    /// CHECK: one-shot temp; SPL owner is the router authority PDA, mint
    /// is passed in (SPL or Token-2022 — v2 legs park Token-2022 base
    /// mints on the temp too)
    #[account(
        mut,
        seeds = [b"pump-hop0", payer.key().as_ref()],
        bump,
    )]
    pub temp: AccountInfo<'info>,

    /// CHECK: must be the router authority PDA — the SPL owner of the temp
    #[account(address = authority_pda::ID)]
    pub authority: AccountInfo<'info>,

    pub token_mint: InterfaceAccount<'info, Mint>,

    pub token_program: Interface<'info, TokenInterface>,

    pub system_program: Program<'info, System>,
}

pub fn create_pump_hop0_temp_handler<'a>(
    ctx: Context<'_, '_, 'a, 'a, CreatePumpHop0TempAccounts<'a>>,
    bump: u8,
) -> Result<()> {
    let temp = ctx.accounts.temp.to_account_info();
    let token_mint = ctx.accounts.token_mint.to_account_info();

    // SPL or Token-2022: the temp's mint rides either program (v2 legs
    // park Token-2022 base mints); anything else is rejected
    require!(
        token_mint.owner == &anchor_spl::token::Token::id()
            || token_mint.owner == &anchor_spl::token_2022::Token2022::id(),
        ErrorCode::InvalidTokenProgram
    );

    // allocate+assign instead of create_account: create_account requires
    // the address to carry 0 lamports, so a 1-lamport transfer from anyone
    // would PERMANENTLY brick this wallet's hop0 temp (the PDA keypair
    // does not exist, the lamports can never be withdrawn). allocate only
    // requires empty data — pre-sent lamports are harmless (they ride
    // along into the close sweep).
    let seeds: &[&[u8]] = &[b"pump-hop0", ctx.accounts.payer.key.as_ref(), &[bump]];
    let ix_allocate = system_instruction::allocate(temp.key, TokenAccount::LEN as u64);
    invoke_signed(&ix_allocate, &[temp.clone()], &[seeds])?;
    let ix_assign = system_instruction::assign(temp.key, token_mint.owner);
    invoke_signed(&ix_assign, &[temp.clone()], &[seeds])?;
    // rent (minus anything already sitting on the address) from the payer
    let rent = Rent::get()?;
    let need = rent
        .minimum_balance(TokenAccount::LEN)
        .saturating_sub(temp.lamports());
    if need > 0 {
        let ix_fund = system_instruction::transfer(
            ctx.accounts.payer.to_account_info().key,
            temp.key,
            need,
        );
        invoke(
            &ix_fund,
            &[ctx.accounts.payer.to_account_info(), temp.clone()],
        )?;
    }

    if temp.lamports() >= rent.minimum_balance(TokenAccount::LEN) && temp.data_len() > 0 {
        // already initialized (same-slot retry): nothing to do
        return Ok(());
    }
    token_interface::initialize_account3(CpiContext::new(
        ctx.accounts.token_program.to_account_info(),
        token_interface::InitializeAccount3 {
            account: temp,
            mint: token_mint,
            authority: ctx.accounts.authority.to_account_info(),
        },
    ))?;

    msg!(
        "pump hop0 temp created: {} (owner {}, mint {})",
        ctx.accounts.temp.key(),
        ctx.accounts.authority.key(),
        ctx.accounts.token_mint.key()
    );
    Ok(())
}
