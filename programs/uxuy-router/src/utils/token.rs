use crate::constants::MIN_SOL_ACCOUNT_RENT;
use crate::error::ErrorCode;
use anchor_lang::prelude::*;
use anchor_lang::solana_program::program::{invoke, invoke_signed};
use anchor_lang::solana_program::system_instruction::transfer;
use anchor_spl::associated_token::{AssociatedToken, create};
use anchor_spl::token::Token;
use anchor_spl::token_2022::spl_token_2022::extension::BaseStateWithExtensions;
use anchor_spl::token_2022::spl_token_2022::{
    self,
    extension::{StateWithExtensions, transfer_fee::TransferFeeConfig},
};
use anchor_spl::token_2022::{self, Token2022};
use anchor_spl::token_2022_extensions;
use anchor_spl::token_interface::{Mint, TokenAccount, TokenInterface};

/// Reject a caller-supplied program id before it is handed a PDA signature
/// or forwarded into a DEX CPI: only the two token programs are valid.
pub fn require_token_program(program: &AccountInfo) -> Result<()> {
    require!(
        program.key() == anchor_spl::token::Token::id()
            || program.key() == anchor_spl::token_2022::Token2022::id(),
        ErrorCode::InvalidTokenProgram
    );
    Ok(())
}

/// Canonical associated-token address. The ATA derivation takes THREE seeds
/// (wallet, token program, mint) — a two-seed derive silently produces a
/// different, unusable address.
pub fn associated_token_address(wallet: &Pubkey, mint: &Pubkey, token_program: &Pubkey) -> Pubkey {
    anchor_spl::associated_token::get_associated_token_address_with_program_id(
        wallet,
        mint,
        token_program,
    )
}

/// Token-account `amount` without an Anchor re-deserialize: the SPL and
/// Token-2022 base layouts both carry it at bytes 64..72. Used where a
/// lamports delta cannot express a token movement (non-native mints).
pub fn token_account_amount(account: &AccountInfo) -> Result<u64> {
    let data = account.try_borrow_data()?;
    require!(data.len() >= 72, ErrorCode::InvalidTokenAccount);
    let mut buf = [0u8; 8];
    buf.copy_from_slice(&data[64..72]);
    Ok(u64::from_le_bytes(buf))
}

pub fn transfer_token<'a>(
    authority: AccountInfo<'a>,
    from: AccountInfo<'a>,
    to: AccountInfo<'a>,
    mint: AccountInfo<'a>,
    token_program: AccountInfo<'a>,
    amount: u64,
    mint_decimals: u8,
    signer_seeds: Option<&[&[&[u8]]]>,
) -> Result<()> {
    // Skip if transferring to self or amount is 0
    if amount == 0 || from.key == to.key {
        return Ok(());
    }
    if let Some(signer_seeds) = signer_seeds {
        token_2022::transfer_checked(
            CpiContext::new_with_signer(
                token_program.to_account_info(),
                token_2022::TransferChecked { from, to, authority, mint },
                signer_seeds,
            ),
            amount,
            mint_decimals,
        )
    } else {
        token_2022::transfer_checked(
            CpiContext::new(
                token_program.to_account_info(),
                token_2022::TransferChecked { from, to, authority, mint },
            ),
            amount,
            mint_decimals,
        )
    }
}

pub fn transfer_sol<'a>(
    from: AccountInfo<'a>,
    to: AccountInfo<'a>,
    lamports: u64,
    signer_seeds: Option<&[&[&[u8]]]>,
) -> Result<()> {
    // Skip if transferring to self or amount is 0
    if lamports == 0 || from.key == to.key {
        return Ok(());
    }
    let ix = transfer(from.key, to.key, lamports);
    if let Some(signer_seeds) = signer_seeds {
        invoke_signed(&ix, &[from, to], signer_seeds)?;
    } else {
        invoke(&ix, &[from, to])?;
    }
    Ok(())
}

/// Transfer SOL ensuring the recipient has rent-exempt balance
/// If the final balance would be below rent exemption, transfers enough to reach it
/// Returns the actual amount transferred
pub fn transfer_sol_with_rent_exemption<'a>(
    from: &AccountInfo<'a>,
    to: &AccountInfo<'a>,
    requested_amount: u64,
    from_seeds: Option<&[&[&[u8]]]>,
) -> Result<u64> {
    // Skip if transferring to self or amount is 0
    if from.key() == to.key() || requested_amount == 0 {
        return Ok(0);
    }

    // Calculate actual transfer amount to ensure receiver rent exemption
    let receiver_current_balance = to.lamports();
    let receiver_final_balance = receiver_current_balance
        .checked_add(requested_amount)
        .ok_or(ErrorCode::CalculationError)?;

    // Determine actual transfer amount
    let actual_transfer_amount = if receiver_final_balance < MIN_SOL_ACCOUNT_RENT {
        // Need to top up to minimum rent
        MIN_SOL_ACCOUNT_RENT
            .checked_sub(receiver_current_balance)
            .ok_or(ErrorCode::CalculationError)?
    } else {
        // Receiver will have enough, use original amount
        requested_amount
    };

    // Check if sender has enough balance
    let sender_balance = from.lamports();
    if sender_balance < actual_transfer_amount {
        msg!("Insufficient sender balance: {} < {}", sender_balance, actual_transfer_amount);
        return Err(ErrorCode::InsufficientBalance.into());
    }

    // Perform transfer using existing transfer_sol function
    transfer_sol(from.to_account_info(), to.to_account_info(), actual_transfer_amount, from_seeds)?;

    msg!(
        "SOL transferred with rent exemption: {} lamports (requested: {}) to {}",
        actual_transfer_amount,
        requested_amount,
        to.key()
    );

    Ok(actual_transfer_amount)
}

pub fn sync_wsol_account<'a>(
    wsol_account: AccountInfo<'a>,
    token_program: AccountInfo<'a>,
    signer_seeds: Option<&[&[&[u8]]]>,
) -> Result<()> {
    if let Some(signer_seeds) = signer_seeds {
        token_2022::sync_native(CpiContext::new_with_signer(
            token_program.to_account_info(),
            token_2022::SyncNative { account: wsol_account.to_account_info() },
            signer_seeds,
        ))
    } else {
        token_2022::sync_native(CpiContext::new(
            token_program.to_account_info(),
            token_2022::SyncNative { account: wsol_account.to_account_info() },
        ))
    }
}

pub fn close_token_account<'a>(
    token_account: AccountInfo<'a>,
    destination: AccountInfo<'a>,
    authority: AccountInfo<'a>,
    token_program: AccountInfo<'a>,
    signer_seeds: Option<&[&[&[u8]]]>,
) -> Result<()> {
    if token_account.get_lamports() == 0 {
        return Ok(());
    }
    if let Some(signer_seeds) = signer_seeds {
        token_2022::close_account(CpiContext::new_with_signer(
            token_program.to_account_info(),
            token_2022::CloseAccount { account: token_account, destination, authority },
            signer_seeds,
        ))
    } else {
        token_2022::close_account(CpiContext::new(
            token_program.to_account_info(),
            token_2022::CloseAccount { account: token_account, destination, authority },
        ))
    }
}

pub fn create_ata_if_needed<'a>(
    authority: &AccountInfo<'a>,
    payer: &AccountInfo<'a>,
    token_account: &AccountInfo<'a>,
    token_mint: &AccountInfo<'a>,
    token_program: &AccountInfo<'a>,
    associated_token_program: &AccountInfo<'a>,
    system_program: &AccountInfo<'a>,
) -> Result<()> {
    if is_token_account_initialized(token_account) {
        return Ok(());
    }
    create(CpiContext::new(
        associated_token_program.to_account_info(),
        anchor_spl::associated_token::Create {
            payer: payer.to_account_info(),
            associated_token: token_account.to_account_info(),
            authority: authority.to_account_info(),
            mint: token_mint.to_account_info(),
            system_program: system_program.to_account_info(),
            token_program: token_program.to_account_info(),
        },
    ))?;
    Ok(())
}

pub fn create_sa_if_needed<'info>(
    payer: &AccountInfo<'info>,
    mint: &InterfaceAccount<'info, Mint>,
    sa_authority: &Option<UncheckedAccount<'info>>,
    token_sa: &mut Option<UncheckedAccount<'info>>,
    token_program: &Option<Interface<'info, TokenInterface>>,
    associated_token_program: &Option<Program<'info, AssociatedToken>>,
    system_program: &Option<Program<'info, System>>,
) -> Result<Option<InterfaceAccount<'info, TokenAccount>>> {
    if token_sa.is_none() {
        return Ok(None);
    }
    require!(sa_authority.is_some(), ErrorCode::SaAuthorityIsNone);
    require!(token_program.is_some(), ErrorCode::TokenProgramIsNone);
    require!(associated_token_program.is_some(), ErrorCode::AssociatedTokenProgramIsNone);
    require!(system_program.is_some(), ErrorCode::SystemProgramIsNone);

    let token_sa = token_sa.as_ref().unwrap();
    if !is_token_account_initialized(token_sa) {
        create(CpiContext::new(
            associated_token_program.as_ref().unwrap().to_account_info(),
            anchor_spl::associated_token::Create {
                payer: payer.to_account_info(),
                associated_token: token_sa.to_account_info(),
                authority: sa_authority.as_ref().unwrap().to_account_info(),
                mint: mint.to_account_info(),
                system_program: system_program.as_ref().unwrap().to_account_info(),
                token_program: token_program.as_ref().unwrap().to_account_info(),
            },
        ))?;
    }
    let token_sa_box = Box::leak(Box::new(token_sa.clone()));
    Ok(Some(InterfaceAccount::<TokenAccount>::try_from(token_sa_box)?))
}

/// Check if the token account is initialized
pub fn is_token_account_initialized(account: &AccountInfo) -> bool {
    // Check if the account has been rented (has allocated space) or is empty
    if account.lamports() == 0 || account.data_is_empty() {
        return false;
    }
    // Check if the account owner is the Token program
    if *account.owner != Token::id() && *account.owner != Token2022::id() {
        return false;
    }
    true
}

/// Calculate the fee for input amount
pub fn get_transfer_fee(mint_info: &AccountInfo, pre_fee_amount: u64) -> Result<u64> {
    if *mint_info.owner == Token::id() {
        return Ok(0);
    }
    let mint_data = mint_info.try_borrow_data()?;
    let mint = StateWithExtensions::<spl_token_2022::state::Mint>::unpack(&mint_data)?;

    let fee = if let Ok(transfer_fee_config) = mint.get_extension::<TransferFeeConfig>() {
        transfer_fee_config.calculate_epoch_fee(Clock::get()?.epoch, pre_fee_amount).unwrap()
    } else {
        0
    };
    Ok(fee)
}

pub fn harvest_withheld_tokens_to_mint<'a>(
    token_program: AccountInfo<'a>,
    token_mint: AccountInfo<'a>,
    token_account: AccountInfo<'a>,
    signer_seeds: Option<&[&[&[u8]]]>,
) -> Result<()> {
    if *token_mint.owner == Token::id() || token_program.key() == Token::id() {
        return Ok(());
    }
    if let Some(signer_seeds) = signer_seeds {
        token_2022_extensions::transfer_fee::harvest_withheld_tokens_to_mint(
            CpiContext::new_with_signer(
                token_program.to_account_info(),
                token_2022_extensions::transfer_fee::HarvestWithheldTokensToMint {
                    token_program_id: token_program,
                    mint: token_mint,
                },
                signer_seeds,
            ),
            vec![token_account.to_account_info()],
        )?;
    } else {
        token_2022_extensions::transfer_fee::harvest_withheld_tokens_to_mint(
            CpiContext::new(
                token_program.to_account_info(),
                token_2022_extensions::transfer_fee::HarvestWithheldTokensToMint {
                    token_program_id: token_program.to_account_info(),
                    mint: token_mint.to_account_info(),
                },
            ),
            vec![token_account.to_account_info()],
        )?;
    }
    Ok(())
}

pub fn associate_convert_token_account<'info>(
    token_account: &AccountInfo<'info>,
) -> Result<InterfaceAccount<'info, TokenAccount>> {
    let account_box = Box::leak(Box::new(token_account.as_ref().to_account_info()));
    InterfaceAccount::<TokenAccount>::try_from(account_box)
        .map_err(|_| ErrorCode::InvalidTokenAccount.into())
}

pub fn is_token_account(account: &AccountInfo) -> bool {
    account.as_ref().owner == &crate::token_program::ID
        || account.as_ref().owner == &crate::token_2022_program::ID
}

pub fn is_system_account(account: &AccountInfo) -> bool {
    account.as_ref().owner == &crate::system_program::ID
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constants::{authority_pda, wsol_program, wsol_sa};
    use anchor_spl::token_2022::spl_token_2022::extension::{
        transfer_fee::TransferFeeAmount, BaseStateWithExtensionsMut, ExtensionType,
        StateWithExtensionsMut,
    };
    use anchor_spl::token_2022::spl_token_2022::state::{Account, AccountState};

    // The ATA derivation carries THREE seeds (wallet, token program, mint).
    // Pinned against the checked-in wsol_sa constant so the two-seed derive
    // that silently produced an unusable address cannot come back.
    #[test]
    pub fn test_associated_token_address_matches_wsol_sa() {
        assert_eq!(
            associated_token_address(
                &authority_pda::ID,
                &wsol_program::ID,
                &anchor_spl::token::Token::id()
            ),
            wsol_sa::ID
        );
    }

    // The v2 hop0-final forward used to read the lamports delta (always 0
    // for a token movement); pin the SPL base-layout offset the replacement
    // relies on.
    #[test]
    pub fn test_token_account_amount_reads_spl_layout() {
        let key = Pubkey::new_unique();
        let owner = anchor_spl::token::Token::id();
        let mut lamports = 0u64;
        let mut data = [0u8; 165];
        data[64..72].copy_from_slice(&1_234_567u64.to_le_bytes());
        let mut binding = &mut data[..];
        let ai = AccountInfo::new(
            &key,
            false,
            true,
            &mut lamports,
            &mut binding,
            &owner,
            false,
            0,
        );
        assert_eq!(token_account_amount(&ai).unwrap(), 1_234_567);
    }

    // The platform-fee path reinterprets caller-supplied fee accounts
    // through associate_convert_token_account before transfer_token_fee
    // debits them: pin that every on-chain account shape deserializes —
    // plain SPL, extension-less Token-2022 (pump-style mints), and
    // Token-2022 grown past 165 bytes by the TransferFeeAmount extension
    // (fee-bearing mints). A 165-exact-only deserializer would reject the
    // extended fee account and brick Token-2022 fee collection.
    #[test]
    pub fn test_associate_convert_token_account_accepts_t2022_shapes() {
        // plain SPL (165 exact): mint 0..32, owner 32..64, amount 64..72,
        // state byte 108 = Initialized
        {
            let key = Pubkey::new_unique();
            let owner = anchor_spl::token::Token::id();
            let mut lamports = 0u64;
            let mut data = vec![0u8; 165];
            data[64..72].copy_from_slice(&1_111u64.to_le_bytes());
            data[108] = AccountState::Initialized as u8;
            let mut binding = &mut data[..];
            let ai = AccountInfo::new(
                &key,
                false,
                true,
                &mut lamports,
                &mut binding,
                &owner,
                false,
                0,
            );
            assert_eq!(associate_convert_token_account(&ai).unwrap().amount, 1_111);
        }

        // Token-2022 without extensions: same base layout under Token-2022
        {
            let key = Pubkey::new_unique();
            let owner = anchor_spl::token_2022::Token2022::id();
            let mut lamports = 0u64;
            let mut data = vec![0u8; 165];
            data[64..72].copy_from_slice(&2_222u64.to_le_bytes());
            data[108] = AccountState::Initialized as u8;
            let mut binding = &mut data[..];
            let ai = AccountInfo::new(
                &key,
                false,
                true,
                &mut lamports,
                &mut binding,
                &owner,
                false,
                0,
            );
            assert_eq!(associate_convert_token_account(&ai).unwrap().amount, 2_222);
        }

        // Token-2022 with the TransferFeeAmount extension: the account data
        // grows past 165 — the exact shape a fee-bearing mint's ATAs carry
        let len = ExtensionType::try_calculate_account_len::<Account>(&[
            ExtensionType::TransferFeeAmount,
        ])
        .unwrap();
        let mut extended = vec![0u8; len];
        {
            let mut state = StateWithExtensionsMut::<Account>::unpack_uninitialized(&mut extended)
                .unwrap();
            state.init_extension::<TransferFeeAmount>(true).unwrap();
            state.base.amount = 3_333;
            state.base.state = AccountState::Initialized;
            state.pack_base();
            state.init_account_type().unwrap();
        }
        assert!(len > 165, "extended account must grow past the base len");
        {
            let key = Pubkey::new_unique();
            let owner = anchor_spl::token_2022::Token2022::id();
            let mut lamports = 0u64;
            let mut binding = &mut extended[..];
            let ai = AccountInfo::new(
                &key,
                false,
                true,
                &mut lamports,
                &mut binding,
                &owner,
                false,
                0,
            );
            assert_eq!(associate_convert_token_account(&ai).unwrap().amount, 3_333);
        }
    }
}
