use anchor_lang::prelude::*;

// pinocchio_log::log! only supports Display-style placeholders (`{}`, `{:.n}`, ...).
// Every logged value here is numeric or boolean, so `{}` renders identically to `{:?}`,
// allowing us to satisfy the macro while keeping the same output as the old msg! calls.
macro_rules! log_msg {
    ($fmt:literal $(, $arg:expr)* $(,)?) => {
        // 256 bytes comfortably cover the longest formatted message (~<200 bytes),
        // avoiding heap allocations while leaving headroom for future fields.
        pinocchio_log::log!(256, $fmt $(, $arg)*);
    };
}

pub fn log_swap_basic_info(
    order_id: u64,
    source_mint: &Pubkey,
    destination_mint: &Pubkey,
    source_owner: &Pubkey,
    destination_owner: &Pubkey,
) {
    if order_id > 0 {
        log_msg!("order_id: {}", order_id);
    }
    source_mint.log();
    destination_mint.log();
    source_owner.log();
    destination_owner.log();
}

pub fn log_swap_balance_before(
    before_source_balance: u64,
    before_destination_balance: u64,
    amount_in: u64,
    expect_amount_out: u64,
    min_return: u64,
) {
    log_msg!(
        "before_source_balance: {}, before_destination_balance: {}, amount_in: {}, expect_amount_out: {}, min_return: {}",
        before_source_balance,
        before_destination_balance,
        amount_in,
        expect_amount_out,
        min_return
    );
}

pub fn log_swap_end(
    after_source_balance: u64,
    after_destination_balance: u64,
    source_token_change: u64,
    destination_token_change: u64,
) {
    log_msg!(
        "after_source_balance: {}, after_destination_balance: {}, source_token_change: {}, destination_token_change: {}",
        after_source_balance,
        after_destination_balance,
        source_token_change,
        destination_token_change
    );
}

pub fn log_commission_info(commission_direction: bool, commission_amount: u64, adjust_amount: u64) {
    log_msg!(
        "commission_direction: {}, commission_amount: {}, commission_adjust_amount: {}",
        commission_direction,
        commission_amount,
        adjust_amount
    );
}

pub fn log_platform_fee_info(amount: u64, adjust_amount: u64, fee_account: &Pubkey) {
    log_msg!("platform_fee_amount: {}, platform_fee_adjust_amount: {}", amount, adjust_amount);
    fee_account.log();
}

pub fn log_trim_fee_info(amount: u64, adjust_amount: u64, fee_account: &Pubkey) {
    log_msg!("trim_fee_amount: {}, trim_fee_adjust_amount: {}", amount, adjust_amount);
    fee_account.log();
}

pub fn log_charge_fee_info(amount: u64, adjust_amount: u64, fee_account: &Pubkey) {
    log_msg!("charge_fee_amount: {}, charge_fee_adjust_amount: {}", amount, adjust_amount);
    fee_account.log();
}

pub fn log_rate_info(commission_rate: u32, platform_fee_rate: u32, trim_rate: Option<u8>) {
    if let Some(trim_rate) = trim_rate {
        log_msg!(
            "commission_rate: {}, platform_fee_rate: {}, trim_rate: {}",
            commission_rate,
            platform_fee_rate,
            trim_rate
        );
    } else {
        log_msg!("commission_rate: {}, platform_fee_rate: {}", commission_rate, platform_fee_rate);
    }
}

pub fn log_rate_info_v3(
    commission_rate: u32,
    platform_fee_rate: Option<u16>,
    trim_rate: Option<u8>,
    commission_direction: bool,
    acc_close_flag: bool,
) {
    let platform_fee_rate_val = platform_fee_rate.unwrap_or(0);
    let trim_rate_val = trim_rate.unwrap_or(0);
    log_msg!(
        "commission_rate: {}, platform_fee_rate: {}, trim_rate: {}, commission_direction: {}, acc_close_flag: {}",
        commission_rate,
        platform_fee_rate_val,
        trim_rate_val,
        commission_direction,
        acc_close_flag
    );
}

pub fn log_fee_accounts_info<'info>(
    commission_account: &Option<AccountInfo<'info>>,
    platform_fee_account: &Option<AccountInfo<'info>>,
) {
    commission_account.as_ref().map(|acc| acc.key()).unwrap_or(Pubkey::default()).log();
    platform_fee_account.as_ref().map(|acc| acc.key()).unwrap_or(Pubkey::default()).log();
}

pub fn log_rate_info_v3_enhanced(
    commission_rate: u32,
    platform_fee_rate: Option<u16>,
    trim_rate: u8,
    charge_rate: u16,
    commission_direction: bool,
    acc_close_flag: bool,
) {
    let platform_fee_rate_val = platform_fee_rate.unwrap_or(0);
    log_msg!(
        "commission_rate: {}, platform_fee_rate: {}, trim_rate: {}, charge_rate: {}, commission_direction: {}, acc_close_flag: {}",
        commission_rate,
        platform_fee_rate_val,
        trim_rate,
        charge_rate,
        commission_direction,
        acc_close_flag
    );
}

pub fn log_claim_info_before(source_balance: u64, destination_balance: u64, amount: u64) {
    log_msg!(
        "before_source_balance: {}, before_destination_balance: {}, amount: {}",
        source_balance,
        destination_balance,
        amount
    );
}

pub fn log_claim_info_after(
    source_balance: u64,
    destination_balance: u64,
    source_token_change: u64,
    destination_token_change: u64,
) {
    log_msg!(
        "after_source_balance: {}, after_destination_balance: {}, source_token_change: {}, destination_token_change: {}",
        source_balance,
        destination_balance,
        source_token_change,
        destination_token_change
    );
}

pub fn log_sa_lamports_info(
    before_sa_lamports: u64,
    after_sa_lamports: u64,
    diff_sa_lamports: u64,
) {
    log_msg!(
        "before_sa_lamports: {}, after_sa_lamports: {}, diff_sa_lamports: {}",
        before_sa_lamports,
        after_sa_lamports,
        diff_sa_lamports
    );
}

#[cfg(all(test, not(target_os = "solana")))]
mod tests {
    /// Helper to format using pinocchio_log's Logger, mirroring log_msg! formatting.
    fn format_with_pinocchio_log<F>(f: F) -> String
    where
        F: FnOnce(&mut pinocchio_log::logger::Logger<256>),
    {
        use pinocchio_log::logger::Logger;
        let mut logger = Logger::<256>::default();
        f(&mut logger);
        String::from_utf8_lossy(&*logger).to_string()
    }

    /// Ensure msg! and log_msg! stay output-compatible for all covered patterns.
    #[test]
    fn msg_and_log_msg_emit_same_bytes() {
        // Simple u64
        let msg_output = format!("order_id: {}", 12345u64);
        let log_msg_output = format_with_pinocchio_log(|logger| {
            logger.append("order_id: ").append(12345u64);
        });
        assert_eq!(msg_output, log_msg_output, "u64 formatting should match");

        // Multiple numeric + bool arguments (log_rate_info style)
        let commission_rate = 500u32;
        let platform_fee_rate_val = 100u16;
        let trim_rate_val = 10u8;
        let commission_direction = true;
        let acc_close_flag = false;

        let msg_output = format!(
            "commission_rate: {}, platform_fee_rate: {}, trim_rate: {}, commission_direction: {}, acc_close_flag: {}",
            commission_rate,
            platform_fee_rate_val,
            trim_rate_val,
            commission_direction,
            acc_close_flag
        );
        let log_msg_output = format_with_pinocchio_log(|logger| {
            logger
                .append("commission_rate: ")
                .append(commission_rate)
                .append(", platform_fee_rate: ")
                .append(platform_fee_rate_val)
                .append(", trim_rate: ")
                .append(trim_rate_val)
                .append(", commission_direction: ")
                .append(commission_direction)
                .append(", acc_close_flag: ")
                .append(acc_close_flag);
        });
        assert_eq!(msg_output, log_msg_output, "multi-argument formatting should match");

        // Boolean formatting
        let msg_output = format!("commission_direction: {}", true);
        let log_msg_output = format_with_pinocchio_log(|logger| {
            logger.append("commission_direction: ").append(true);
        });
        assert_eq!(msg_output, log_msg_output, "bool formatting should match");

        // String interpolation case (log_dex_adapter_entry analogue)
        let dex_name = "Raydium";
        let amount_in = 1_000_000u64;
        let offset = 42usize;

        let msg_output = format!("Dex::{} amount_in: {}, offset: {}", dex_name, amount_in, offset);
        let log_msg_output = format_with_pinocchio_log(|logger| {
            logger
                .append("Dex::")
                .append(dex_name)
                .append(" amount_in: ")
                .append(amount_in)
                .append(", offset: ")
                .append(offset);
        });
        assert_eq!(msg_output, log_msg_output, "string + numeric formatting should match");

        // Multiple u64s (log_swap balance style)
        let before_source_balance = 1_000_000u64;
        let before_destination_balance = 2_000_000u64;
        let amount_in = 500_000u64;
        let expect_amount_out = 450_000u64;
        let min_return = 400_000u64;

        let msg_output = format!(
            "before_source_balance: {}, before_destination_balance: {}, amount_in: {}, expect_amount_out: {}, min_return: {}",
            before_source_balance,
            before_destination_balance,
            amount_in,
            expect_amount_out,
            min_return
        );
        let log_msg_output = format_with_pinocchio_log(|logger| {
            logger
                .append("before_source_balance: ")
                .append(before_source_balance)
                .append(", before_destination_balance: ")
                .append(before_destination_balance)
                .append(", amount_in: ")
                .append(amount_in)
                .append(", expect_amount_out: ")
                .append(expect_amount_out)
                .append(", min_return: ")
                .append(min_return);
        });
        assert_eq!(msg_output, log_msg_output, "multiple u64 formatting should match");
    }

    /// Validate `{}` and `{:?}` equivalence for numeric/bool types noted in the comment.
    #[test]
    fn display_and_debug_format_equivalence() {
        // Primitive coverage
        for value in [12345u64 as u128, 500u32 as u128, 100u16 as u128, 10u8 as u128] {
            let display = format!("value: {}", value);
            let debug = format!("value: {:?}", value);
            assert_eq!(display, debug, "Primitives should be identical for {{}} and {{:?}}");
        }

        for value in [true, false] {
            let display = format!("value: {}", value);
            let debug = format!("value: {:?}", value);
            assert_eq!(display, debug, "bool should be identical for {{}} and {{:?}}");
        }

        let display = format!("value: {}", 42usize);
        let debug = format!("value: {:?}", 42usize);
        assert_eq!(display, debug, "usize should be identical for {{}} and {{:?}}");

        // msg! with {:?} vs log_msg! with {}
        let commission_rate = 500u32;
        let platform_fee_rate_val = 100u16;
        let trim_rate_val = 10u8;
        let commission_direction = true;
        let acc_close_flag = false;

        let msg_debug = format!(
            "commission_rate: {:?}, platform_fee_rate: {:?}, trim_rate: {:?}, commission_direction: {:?}, acc_close_flag: {:?}",
            commission_rate,
            platform_fee_rate_val,
            trim_rate_val,
            commission_direction,
            acc_close_flag
        );
        let log_msg_display = format_with_pinocchio_log(|logger| {
            logger
                .append("commission_rate: ")
                .append(commission_rate)
                .append(", platform_fee_rate: ")
                .append(platform_fee_rate_val)
                .append(", trim_rate: ")
                .append(trim_rate_val)
                .append(", commission_direction: ")
                .append(commission_direction)
                .append(", acc_close_flag: ")
                .append(acc_close_flag);
        });
        assert_eq!(
            msg_debug, log_msg_display,
            "msg! with {{:?}} should match log_msg! with {{}} for numeric/bool arguments"
        );

        // Mixed fields
        let order_id = 12345u64;
        let is_active = true;
        let amount = 1_000_000u64;

        let msg_display =
            format!("order_id: {}, is_active: {}, amount: {}", order_id, is_active, amount);
        let msg_debug =
            format!("order_id: {:?}, is_active: {:?}, amount: {:?}", order_id, is_active, amount);
        assert_eq!(msg_display, msg_debug, "msg! should emit same bytes for {{}} and {{:?}}");

        let log_msg_output = format_with_pinocchio_log(|logger| {
            logger
                .append("order_id: ")
                .append(order_id)
                .append(", is_active: ")
                .append(is_active)
                .append(", amount: ")
                .append(amount);
        });
        assert_eq!(msg_display, log_msg_output, "log_msg! with {{}} should match msg! with {{}}");
        assert_eq!(msg_debug, log_msg_output, "log_msg! with {{}} should match msg! with {{:?}}");
    }
}
