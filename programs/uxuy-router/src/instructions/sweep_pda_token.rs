use crate::error::ErrorCode;
use crate::{SA_AUTHORITY_SEED, authority_pda, claim_authority};
use anchor_lang::prelude::*;
use anchor_lang::solana_program::program_pack::Pack;
use anchor_spl::token::spl_token::state::Account as SplTokenAccount;
use anchor_spl::token::Token;

// Ops-gated sweep of value held by the router authority PDA (the cashback
// sink — the PDA-uva quote ATAs credited by pump's buy cashback and the
// claim_cashback/claim_token_incentives flows land in PDA-owned accounts
// that only this program can move). claim_authority signs; the source must
// be an SPL token account owned by the authority PDA.
#[derive(Accounts)]
pub struct SweepPdaTokenAccounts<'info> {
    /// CHECK: ops sweep key (same gate as claim)
    #[account(address = claim_authority::id() @ ErrorCode::InvalidSigner)]
    pub signer: Signer<'info>,

    /// CHECK: must be an SPL token account owned by the authority PDA
    #[account(mut)]
    pub source_token_account: AccountInfo<'info>,

    /// CHECK: any destination token account of the same mint
    #[account(mut)]
    pub destination_token_account: AccountInfo<'info>,

    /// CHECK: must be the router authority PDA — the transfer signer
    #[account(address = authority_pda::ID)]
    pub authority: AccountInfo<'info>,

    pub token_program: Program<'info, Token>,
}

pub fn sweep_pda_token_handler<'a>(
    ctx: Context<'_, '_, 'a, 'a, SweepPdaTokenAccounts<'a>>,
    amount: u64,
) -> Result<()> {
    let data = ctx.accounts.source_token_account.try_borrow_data()?;
    let ta = SplTokenAccount::unpack(&data)
        .map_err(|_| ErrorCode::InvalidSourceTokenAccount)?;
    require_keys_eq!(
        ta.owner,
        ctx.accounts.authority.key(),
        ErrorCode::InvalidSwapAuthority
    );

    anchor_spl::token::transfer(
        CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            anchor_spl::token::Transfer {
                from: ctx.accounts.source_token_account.to_account_info(),
                to: ctx.accounts.destination_token_account.to_account_info(),
                authority: ctx.accounts.authority.to_account_info(),
            },
            SA_AUTHORITY_SEED,
        ),
        amount,
    )?;

    msg!(
        "pda token swept: {} -> {} amount {}",
        ctx.accounts.source_token_account.key(),
        ctx.accounts.destination_token_account.key(),
        amount
    );
    Ok(())
}
