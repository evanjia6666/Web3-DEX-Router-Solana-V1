use crate::error::ErrorCode;
use crate::{authority_pda, token_program};
use anchor_lang::prelude::*;
use anchor_lang::solana_program::program::invoke_signed;
use anchor_lang::solana_program::system_instruction;
use anchor_spl::token::Token;
use anchor_spl::token::TokenAccount;
use anchor_spl::token_interface::{self, Mint};

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
    /// is passed in and must be an SPL (not Token-2022) mint
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

    #[account(address = token_program::id())]
    pub token_program: Program<'info, Token>,

    pub system_program: Program<'info, System>,
}

pub fn create_pump_hop0_temp_handler<'a>(
    ctx: Context<'_, '_, 'a, 'a, CreatePumpHop0TempAccounts<'a>>,
    bump: u8,
) -> Result<()> {
    let temp = ctx.accounts.temp.to_account_info();
    let token_mint = ctx.accounts.token_mint.to_account_info();

    // SPL-only: the pump verifiers and the hop0 close ride the plain SPL
    // program; Token-2022 hop0 inputs are rejected at compose already and
    // this re-checks it on-chain
    require!(
        token_mint.owner == &token_program::ID,
        ErrorCode::InvalidTokenProgram
    );

    // rent funded by the payer inside this instruction; system
    // create_account fails on a funded/in-use account, which doubles as
    // the same-slot collision guard
    let rent = Rent::get()?;
    let lamports = rent.minimum_balance(TokenAccount::LEN);
    let seeds: &[&[u8]] = &[b"pump-hop0", ctx.accounts.payer.key.as_ref(), &[bump]];
    let ix_create = system_instruction::create_account(
        ctx.accounts.payer.to_account_info().key,
        temp.key,
        lamports,
        TokenAccount::LEN as u64,
        &token_program::ID,
    );
    invoke_signed(
        &ix_create,
        &[ctx.accounts.payer.to_account_info(), temp.clone()],
        &[seeds],
    )?;

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
