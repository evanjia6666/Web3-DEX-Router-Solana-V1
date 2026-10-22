# UXUY Solana Router — 集成手册（面向 `uxuyprotocol/dex` Go 侧）

本分支（`uxuy-router`）把 OKX 开源的 DEX Router V1（MIT）裁剪并改造为 uxuy 专属路由合约。
上游源码：[okxlabs/DEX-Router-Solana-V1](https://github.com/okxlabs/DEX-Router-Solana-V1)（其链上部署体为 `6m2CDdhRgxpH4WjvdzxAYbGxwdGUz5MziiL5jek2kBma`，附 OKX 审计报告 `audit/` 目录）。
本合约**只做执行**：询价、路径搜索仍在链下（`dex_offlineQuoteSol` / `dex_offlineSwapSol` 的引擎），合约负责原子执行、逐跳账户校验、余额差结算与 `min_return` 兜底。

## 1. 部署身份（本地生成，主网部署前请评审）

| 项 | 值 | 说明 |
|---|---|---|
| Program ID | `AN6zjJfTD5Cpj2x56sWsAod1Jb5viiQH4Xcg3tm5DgNY` | keypair：`target/deploy/uxuy_router-keypair.json`（gitignore 内，务必备份到密钥管理系统） |
| authority PDA | `5rvVfdVCgSQ9AyTTctN1MPGk5brWVEtvMfPKtcesAuVG` | `PDA(["uxuy_sa"], program_id)`，bump **255**，中间跳代币账户与手续费转账的签名者 |
| wsol_sa | `G6KLoMzx47Nh1xJLtzyGiFXiwH688Z8LEwHtFv4xBPyj` | authority PDA 的 WSOL ATA（`wrap_unwrap_v3` TOB 模式用） |
| claim_authority | `J8NpT3tG57ggTbr4ek8qwapG9SeGnTfXmNZn6ncjuRsG` | 允许调用 `claim` 的 uxuy 运营钱包（keypair：`target/deploy/uxuy_claim_authority-keypair.json`） |
| 种子 | `SEED_SA="uxuy_sa"`、`SEED_TEMP_WSOL="temp_wsol"` | 见 `src/constants.rs` |
| 临时账户 | `create_token_account_with_seed` | seed 由链下指定（推荐复用 `temp_wsol + 随机数` 模式），owner 为用户、authority 为程序 PDA |
| 构建产物 | `target/deploy/uxuy_router.so`（约 850 KB，原版 1.9 MB） | `anchor build`；IDL：`target/idl/uxuy_router.json` |

> 注意：换 program keypair 时，`authority_pda`、`BUMP_SA`、`wsol_sa` 三处必须按新 ID 重新推导（`constants.rs`），否则 PDA 签名全部失败。

## 2. 指令集（12 个，与链上 OKX V1 同构）

| 指令 | 用途 | 对应 OKX API 语义 |
|---|---|---|
| `swap` | 用户直接签名的基本兑换 | swap |
| `proxy_swap` | 经由 PDA 托管（SA）账户兑换 | proxy swap |
| `swap_v3` | 佣金 + 平台费（ToC：佣金可从输入/输出端抽） | feePercent / fromTokenReferrer / toTokenReferrer |
| `swap_v3_with_cpi_event` | 同上 + CPI event | 事件订阅场景 |
| `swap_tob_v3` | ToC 佣金 + trim（正滑点抽成）+ 平台费 | positiveSlippagePercent |
| `swap_tob_v3_with_receiver` | 同上 + 指定收款人（`sol_receiver`，配合 `acc_close_flag` 关户退租、原生 SOL 直付） | swapReceiverAddress + closeAuthorityAddress |
| `swap_tob_v3_enhanced` | 再加 charge（对报价额抽费） | 白名单功能 |
| `wrap_unwrap_v3` / `wrap_unwrap_v3_with_receiver` | SOL↔wSOL 互转并抽费 | 兑 SOL 输出的封装 |
| `create_token_account` / `create_token_account_with_seed` | 创建临时 token 账户（程序可关闭） | 响应中的 createTokenAccountList |
| `claim` | 运营从 PDA 托管临时账户提残留资金 | — |

## 3. SwapArgs 与 uxuy 路由的映射

```rust
SwapArgs {
    amount_in: u64,          // 整单输入
    expect_amount_out: u64,  // 链下预估产出（>= min_return）
    min_return: u64,         // 兜底：整条路径余额差 < min_return 则整笔回滚
    amounts: Vec<u64>,       // 一级拆分：uxuy 目前恒为 [amount_in]
    routes: Vec<Vec<Route>>, // 每份资金一条多跳路径（<= 3 跳）
}
Route { dexes: Vec<Dex>, weights: Vec<u8> }  // 每跳可并行拆分；uxuy 恒为单池 [dex], [100]
```

uxuy 引擎的路由模型是"单路径、每跳一个最优池、最多 3 跳"，映射规则：

- `amounts = [amount_in]`，`routes = [[Route { dexes: [D_i], weights: [100] } for 每跳 i]]`；
- `min_return` = 现有 `r.MinAmount`（引擎已按 slippagePercent 算好）；上线 router 后 `haircutIntermediate` 可下线；
- 约束：`Σamounts == amount_in`、每跳 `Σweights == 100`、`MAX_HOPS = 3`。

## 4. Dex 变体 ↔ uxuy builder 映射 + remaining_accounts 布局

**拼装规则**：各跳账户按顺序首尾相接塞进 `remaining_accounts`（程序内部用 offset 游标逐段消费）；每段的 `swap_source_token` / `swap_destination_token` =
第一跳用用户源账户 / 最后 一跳用用户目标账户，**中间跳一律用程序 PDA 持有的临时账户**（交易前用 `create_token_account_with_seed` 建好，authority 为 authority PDA）；`swap_authority_pubkey` = authority PDA。
字段顺序必须与下表完全一致（= `src/adapters/*.rs` 中对应 struct 的字段序）：


### RaydiumSwap — Raydium AMM v4 `swapBaseIn`（19 账户）
`dex_program_id, swap_authority_pubkey, swap_source_token, swap_destination_token, token_program, amm_id, amm_authority, amm_open_orders, amm_target_orders, pool_coin_token_account, pool_pc_token_account, serum_program_id, serum_market, serum_bids, serum_asks, serum_event_queue, serum_coin_vault_account, serum_pc_vault_account, serum_vault_signer`

### RaydiumSwapV2 — Raydium AMM v4 精简布局（9 账户）
`dex_program_id, swap_authority_pubkey, swap_source_token, swap_destination_token, token_program, amm_id, amm_authority, pool_coin_token_account, pool_pc_token_account`

### RaydiumCpmmSwap — Raydium CPMM `swapBaseInput`（14 账户）
`dex_program_id, swap_authority_pubkey, swap_source_token, swap_destination_token, authority, amm_config, pool_state, input_vault, output_vault, input_token_program, output_token_program, input_token_mint, output_token_mint, observation_state`

### RaydiumClmmSwap — Raydium CLMM `swap`（14 账户）
`dex_program_id, swap_authority_pubkey, swap_source_token, swap_destination_token, amm_config_id, pool_id, input_vault, output_vault, observation_id, tick_array0, ex_bitmap, tick_array1, tick_array2, token_program`

### RaydiumClmmSwapV2 — Raydium CLMM `swapV2`（18 账户，Token-2022 兼容）
`dex_program_id, swap_authority_pubkey, swap_source_token, swap_destination_token, amm_config_id, pool_id, input_vault, output_vault, observation_id, token_program, token_program_2022, memo_program, input_vault_mint, output_vault_mint, ex_bitmap, tick_array0, tick_array1, tick_array2`

### Whirlpool — Orca Whirlpool `swap`（12 账户）
`dex_program_id, swap_authority_pubkey, swap_source_token, swap_destination_token, token_program, whirlpool, token_vault_a, token_vault_b, tick_array0, tick_array1, tick_array2, oracle`

### WhirlpoolV2 — Orca Whirlpool `swapV2`（16 账户，Token-2022 兼容）
`dex_program_id, swap_authority_pubkey, swap_source_token, swap_destination_token, token_program_a, token_program_b, memo_program, whirlpool, token_mint_a, token_mint_b, token_vault_a, token_vault_b, tick_array0, tick_array1, tick_array2, oracle`

### PancakeSwapV3Swap — PancakeSwap CLMM `swap`（14 账户，Raydium 布局分支）
`dex_program_id, swap_authority_pubkey, swap_source_token, swap_destination_token, amm_config, pool_state, input_vault, output_vault, observation_state, token_program, tick_array0, ex_bitmap, tick_array1, tick_array2`

### PancakeSwapV3SwapV2 — PancakeSwap CLMM `swapV2`（18 账户）
`dex_program_id, swap_authority_pubkey, swap_source_token, swap_destination_token, amm_config, pool_state, input_vault, output_vault, observation_state, token_program, token_program_2022, memo_program, input_vault_mint, output_vault_mint, ex_bitmap, tick_array0, tick_array1, tick_array2`

### MeteoraDlmm — Meteora DLMM `swap`（18 账户）
`dex_program_id, swap_authority_pubkey, swap_source_token, swap_destination_token, lb_pair, bin_array_bitmap_extension, reserve_x, reserve_y, token_x_mint, token_y_mint, oracle, host_fee_in, token_x_program, token_y_program, event_authority, bin_array0, bin_array1, bin_array2`

### MeteoraDlmmSwap2 — Meteora DLMM `swap2`（19 账户）
`dex_program_id, swap_authority_pubkey, swap_source_token, swap_destination_token, lb_pair, bin_array_bitmap_extension, reserve_x, reserve_y, token_x_mint, token_y_mint, oracle, host_fee_in, token_x_program, token_y_program, memo_program, event_authority, bin_array0, bin_array1, bin_array2`

### PumpfunammBuy3 — PumpSwap `buy_exact_quote_in`（23 账户，含 `track_volume=OptionBool{true}` 尾字节）
`dex_program_id, swap_authority_pubkey, swap_source_token, swap_destination_token, pool, global_config, base_mint, quote_mint, pool_base_token_account, pool_quote_token_account, protocol_fee_recipient, protocol_fee_recipient_token_account, base_token_program, quote_token_program, system_program, associated_token_program, event_authority, coin_creator_vault_ata, coin_creator_vault_authority, global_volume_accumulator, user_volume_accumulator, fee_config, fee_program`

2026-10-21 起首买租金由 authority 常备余额承担：`user_volume_accumulator` 是 per-user 账户（种子只含 user；PDA 场景 user 槽即 authority），首建租金 1,346,200 由 pump 向 user 槽收取。**cashback 采集（P2-3，随 Phase-2）**：AMM buy 段在尾三槽后多带第 4 尾槽 = **uva 的 quote-mint ATA**（`ATA(uva(authority), quoteMint)`，writable，窗口 26→27）——pump 对 cashback 币自动把返利存入该账户（首次自动初始化，租金走 user 槽=authority 常备）；claim 无需用户签名（`claim_cashback` 任意人可触发 → 落到 user=PDA 的 canonical wSOL ATA），扫出走既有 **`claim`** 指令（token::authority=sa_authority 约束 + Interface 支持 T2022，覆盖 PDA 持有账户；原计划的 sweep_pda_token 因与其重复已移除）；`claim_token_incentives`（任意 payer 签名）可由运维钱包直接调用。曲线 buy 的 IDL 无此钩子（仅 sell 有 [uva, curve_v2] 现金尾），曲线侧暂不采集。
**2026-10-22 起 AMM buy 的 user 一律 = authority PDA（含 hop0）**：hop0 输入经 PDA 持有账户（native→PDA 持有的 tempIn；token→每笔 PDA temp + 钱包 ATA 转入），before_invoke 在累计器未建（lamports=0 且 data_len=0）时要求 authority 余额 ≥ `PUMPFUNAMM_FUND_HEADROOM`（6M，覆盖 pump 其他可能的 user 槽首建费用：creator-vault / fee-recipient quote ATA 各约 1,513,840），不足报 InsufficientFunds（运维需预置/补充 authority SOL）；after_invoke 先关闭 hop0 已排空的 PDA temp（租金退 payer；custody 只出现在 hop≥1，构造上不可误伤），再把异常盈余扫回 payer，缺口=租金由 authority 吸收，payer 永不为租金转账（钱包净成本=0）。段账户表 = 23 声明账户 + 尾部四槽（pool_v2/buyback/buyback-ATA/cashback sink），窗口 26→27。

**2026-10-20 实测更新**：pump 程序 2026-10 升级后 `BuyExactQuoteIn` 已不再向 user 槽收首建租金（测试网 live simulate：accumulator 未出生 + authority 0 lamports 直接成交，日志无任何租金事件；SOL→PAID 单跳与 USDC→SOL→PAID 三跳全绿）。余额要求保留为防御性闸门（若 pump 恢复收费，常备余额直接吸收）。

### PumpfunammSell3 — PumpSwap `sell`（21 账户）
`dex_program_id, swap_authority_pubkey, swap_source_token, swap_destination_token, pool, global_config, base_mint, quote_mint, pool_base_token_account, pool_quote_token_account, protocol_fee_recipient, protocol_fee_recipient_token_account, base_token_program, quote_token_program, system_program, associated_token_program, event_authority, coin_creator_vault_ata, coin_creator_vault_authority, fee_config, fee_program`

### PumpfunBuy — pump.fun 曲线 `buy_exact_sol_in`（20 账户，2026-10 新增，ordinal 15）
`dex_program_id, swap_authority_pubkey, swap_source_token（一次性 wSOL 临时，会被 close）, swap_destination_token, global, fee_recipient, mint, bonding_curve, associated_bonding_curve, system_program, src_token_program（SPL，close 用）, base_token_program（曲线币，常为 Token-2022）, creator_vault, event_authority, global_volume_accumulator, user_volume_accumulator, fee_config, fee_program, buyback_a, buyback_b`

原生 SOL 桥接（**2026-10-22 起 pump user 一律 = authority PDA，含 hop0**）：before_invoke 把 src wSOL 临时账户 close 进 authority，CPI 以 `spendable_sol_in=amount_in, min_tokens_out=1, track_volume 缺省 None` 执行，after_invoke 把临时账户垫付租金**原额**退给路由 payer（出资方=同一用户钱包，净零）；**合约在 CPI 前强制 `temp.lamports() >= amount_in`**——空 temp 不能再让 authority 常备余额替本路由出资；hop0 的 tempIn 由 Go 创建为 **PDA 持有**（钱包出资租金），合约用 `before_check_pda_user` 校验（authority==PDA 且拥有 source），hop0 的 CPI 由合约以 PDA 种子签名。首买租金由 authority 常备余额承担：`track_volume=None` 并不阻止 2026-10 程序建 per-user 累计器（主网实勘 4Suz3jfC…：累计器被创建并向 user 槽收 1,346,200），故累计器或 creator_vault（新币首买 890,880）任一未建时 before_invoke 要求 authority ≥ `PUMPFUN_FUND_HEADROOM`（2.4M，覆盖两者合计 2,237,080；运维预置），不足报 InsufficientFunds；CPI 实收租金从 authority 扣减、事后不向 payer 追偿。buyback 双尾槽是 2026-10 程序强制要求（第二槽须授权金库；两个槽都传 5YxQFdt…，主网模拟已证）。fee_recipient 钉死 pump 默认 7VtfL8…（按币授权：自设分润的币拒收，路由换次优）。

### PumpfunSell — pump.fun 曲线 `sell`（16 账户，2026-10 新增，ordinal 16）
`dex_program_id, swap_authority_pubkey, swap_source_token, swap_destination_token（wSOL）, global, fee_recipient, mint, bonding_curve, associated_bonding_curve, system_program, creator_vault, base_token_program, wsol_program（SPL，wrap 用）, event_authority, fee_config, fee_program`

CPI 以 `amount, min_sol_output=1` 执行（无尾部账户），after_invoke 把 authority 收到的原生 SOL **实额**转入 dst 并 `sync_native`。不需要一次性临时账户（dst 可为托管 wSOL）。

### PumpfunBuyV2 — pump.fun 曲线 `buy_exact_quote_in_v2`（28 账户，2026-10 新增，ordinal 17）
`dex_program_id, swap_authority_pubkey, swap_source_token（canonical ATA(authority, quote)——pump 校验）, swap_destination_token（base 出账，仅校验 owner）, global, base_mint, quote_mint, base_token_program, quote_token_program, associated_token_program, fee_recipient, associated_quote_fee_recipient, buyback_fee_recipient, associated_quote_buyback_fee_recipient, bonding_curve, associated_base_bonding_curve, associated_quote_bonding_curve, creator_vault, associated_creator_vault, sharing_config, global_volume_accumulator, user_volume_accumulator, associated_user_volume_accumulator, fee_config, fee_program, system_program, event_authority, program, user_edge（window-only：hop0 末跳的用户边账户，中段填 6EF8rr；不进 CPI）`

v2 代币结算：quote 侧走 token 账户（USDC/USDT 曲线，或 wSOL 包装路径——legacy System-quote 曲线也接受 wSOL，链上实证）。无原生 SOL 桥接；首建租金（uva+其 quote ATA+creator-vault ATA，最坏 ~6M）由 authority 常备余额承担：无 payer 垫付/扫回，uva 未建时链上强制 ≥ `PUMPFUN_V2_FUND_HEADROOM`（6M），缺口=租金由 authority 吸收，异常盈余扫回 payer。args = (spendable_quote_in=amount_in, min_tokens_out=1)，无尾部账户（buyback 对是声明账户，官方 7VtfL8/5YxQFdt 已模拟验证授权）。

### PumpfunSellV2 — pump.fun 曲线 `sell_v2`（27 账户，2026-10 新增，ordinal 18）
同 buy_v2 布局减 global_volume_accumulator。`swap_source_token`（base 入账，仅校验 owner）→ `swap_destination_token`（canonical ATA(authority, quote)）。args = (amount, min_quote_out=1)。**形状约束（compose 强制）**：quote 目的地是 hop authority 的 canonical ATA —— v2 sell 只能是 hop0 末跳无 receiver，或中间跳（hop≥1 末跳禁止：PDA 的 ATA ≠ 用户收款账户）。

### ByrealClmm — Byreal `swap_v2`（24 账户，含 payer；程序另有 `swap_v3_dyn`，动态费池仅接受后者）
`dex_program_id, swap_authority_pubkey, swap_source_token, swap_destination_token, payer, amm_config, pool_state, input_token_account, output_token_account, input_vault, output_vault, observation_state, token_program, token_program_2022, memo_program, input_vault_mint, output_vault_mint, tickarray_bitmap_extension, tick_array0..tick_array5`

### ByrealClmm2 — Byreal `swap_v2` 新布局（21 账户；同样不适用于动态费池）
`dex_program_id, swap_authority_pubkey, swap_source_token, swap_destination_token, amm_config, pool_state, input_vault, output_vault, observation_state, token_program, token_program_2022, memo_program, input_vault_mint, output_vault_mint, tickarray_bitmap_extension, tick_array0..tick_array5`


uxuy Go 侧 `swapper/sol/instr/build.go` 的 builder 与上表的对应关系：
`buildWhirlpoolSwap`→`WhirlpoolV2`、Raydium CLMM `swapV2`→`RaydiumClmmSwapV2`、Pancake `swapV2`→`PancakeSwapV3SwapV2`、
`swapBaseInput`→`RaydiumCpmmSwap`、DLMM `swap2`→`MeteoraDlmmSwap2`、Raydium v4 `swapBaseIn`→`RaydiumSwap`/`RaydiumSwapV2`、
PumpSwap `buy_exact_quote_in`/`sell`→`PumpfunammBuy3`/`PumpfunammSell3`（上游的 Buy/Sell/Buy2 旧布局对当前 pump_amm 程序已失效，已在枚举中移除）、Byreal `swap_v3_dyn`→`ByrealClmm`（或新池 `ByrealClmm2`）。
builder 里已有的账户布局知识与上表同源，改造成"去掉用户侧 token 账户、换 PDA 临时账户、按段拼接"即可。

## 5. commission_info 位布局（u32）

| 位 | 含义 |
|---|---|
| bit 31 | `commission_direction`：1=从输入端抽佣，0=从输出端 |
| bit 30 | `acc_close_flag`：目标临时账户成交后关闭（配合 `sol_receiver` 退租+直付原生 SOL） |
| bit 29..0 | `commission_rate`（万分比，上限 10%） |

`swap_tob_v3*` 的 `trim_rate`（u8，千分比，上限 100=10%）：对**正滑点部分**抽成；`platform_fee_rate`（u16，万分比）。

## 6. 复刻目标交易的模式（SOL 输出 + 指定接收人 + 正滑点归集）

参照链上样本 `5PCjTpXz...kxPa`（OKX V2 `create_token_account` + `swap_tob_with_receiver`），用本合约等价实现：

1. `ComputeBudget`×2（CU 上限 + 优先费）；
2. System transfer：feePayer 垫付临时账户租金（2,039,280 lamports/账户）；
3. `create_token_account_with_seed`：建带 close authority 的 wSOL 临时账户；
4. `swap_tob_v3_with_receiver(args, commission_info, trim_rate, platform_fee_rate, sol_receiver=Some(rx))`（order_id 已并入 args）：
   - `acc_close_flag=true` 时 `destination_token_program` 槽必须传真实 SPL Token 程序（ProgramID 填充=Option::None，收尾 close 处报 6050 `DestinationTokenProgramIsNone`）；
   - 收尾资金流（对拍 5PCjTpXz 的 lamport 审计）：关闭临时账户，**租金退回 feePayer（A）**，实际产出以原生 SOL 转给 `rx` —— 接收人拿产出、不含租金（边界：低于租金豁免线的接收地址会被 `transfer_sol_with_rent_exemption` 补足到豁免线，补足由付款人出）；
   - 末尾 `min_return` 余额差校验（min_return 由 args.slippage 链上推导），不达标整笔回滚。

## 7. 部署与运维

```bash
anchor build                 # 产物 target/deploy/uxuy_router.so + target/idl/uxuy_router.json
anchor deploy --provider.cluster devnet   # 先 devnet 全 DEX 矩阵 e2e
anchor deploy --provider.cluster mainnet  # upgrade authority 建议用 Squads 多签
```

- 交易组装保持 v1-only（SIMD-0385）可行（单条 router 指令账户数大幅下降）；ALT 作为二期优化；
- `simulate:true` 闸门照旧保留，验收标准 = simulate 的 token balance 变化与旧纯指令路径逐字段对拍；
- **authority PDA 常备 SOL 水位（2026-10-21 起）**：pump 首买租金由 authority 常备余额承担——曲线累计器/creator-vault 未建时链上强制 ≥ `PUMPFUN_FUND_HEADROOM`（2.4M），AMM 闸门 ≥ `PUMPFUNAMM_FUND_HEADROOM`（6M），v2 闸门同 6M（uva 未建时）。部署/升级后向 authority PDA 转 ≥ 0.01 SOL 并监控（每次 pump 首建租事件扣减一次），低于水位前对应 pump 路径会以 InsufficientFunds 拒单；`claim` 的 SOL 分支同样保留 `CLAIM_SOL_RESERVE`（= 6M，即最大的 pump 首建闸门）后才可扫出，避免一次常规领费把水位打穿。
- **pump PDA-user 统一的耦合部署（2026-10-22）**：新 Go 的 hop0 pump 形状会被旧合约以 `ConstraintTokenOwner 2015` 干净拒绝（预检失败、引擎换次优路由），旧 Go 的钱包形状也会被新合约拒绝——合约升级与 gdex 重部署需在同一低峰窗口连做（先合约后 gdex，间隔期 pump 路由报价自动回退）；**Phase-2 部署同样是耦合窗口**（新 `create_pump_hop0_temp` 指令在旧合约上 InstructionFallbackNotFound 101 干净失败、旧 Go 的钱包 temp 在新合约上过不了 hop0 派生校验；**AMM 窗口 26→27（第 4 尾槽）与 v2 窗口 +1（user-edge 尾槽）同样与旧 Go 互斥**——三处布局变更都要求 gdex 与合约同窗口）；**P2-2 已实现（v2 hop0 统一，随本批部署）**：v2 buy/sell 的 authority/user 一律 = PDA；quote 侧走 PDA 的 canonical quote ATA（compose 的 v2QuoteATAs 幂等创建承担原"预热"职责——每路由按需建、租金一次摊销；buy 输入：钱包 ATA 顶层转入——**建议改走 proxy 的 `source_token_sa` 入金**（合约用 `token_account_amount` 校验本次入金差值 == amount_in）；顶层转入时 canonical ATA 的存量无法在链上与本笔入金区分，Go 需保证每笔前后归零；sell 输出：落 canonical 后由 after-hook 精确转发到用户边账户）；base 侧走可验证一次性 temp（create_pump_hop0_temp 已放宽 T2022）；窗口各 +1 个 window-only user-edge 尾槽（不进 CPI，program-id 为中段填充），hop0 派生验证：buy 源 == canonical ATA、buy 终跳目的/buy 输出 temp == P2-1 派生、sell 源 == P2-1 temp；转发金额按 **token 账户 `amount`** 计量（lamports 只有 native mint 才与余额 1:1，非 wSOL 上恒为 0，不能用 lamports 差值）：buy 侧 `from` 是 payer 独占的一次性 base temp，**整余额转发**（第三方投毒转入也不能让 close 失败）；sell 侧 `from` 是共享的 canonical quote ATA，**只转本跳增量**（存量归平台）；
- 上线顺序建议：devnet e2e → mainnet 小额灰度（按 API key）→ 全量；
- 本仓库相对上游的变更（裁剪 50 个 adapter、换 seed/ID、地址模块改为 `from_str_const`）属于审计增量范围，主网前补一轮审计。

## 8. 与 OKX V2 的差异（为什么保持 V1 模型）

V2（`proVF4...`）把 `min_return` 改为链上由 `expect_amount_out × slippage` 推导、路由扁平化（`weight` u16 bps + `index`）、增加 TokenLedger/RFQ 验签。
uxuy 当前为 exactIn + AMM 池报价，V1 模型 1:1 够用且实现更简单；后续若接 RFQ 流动性，可参照 `sol_rfq` adapter（上游）补验签腿。

## 9. Adapter ↔ 链上 IDL 验证记录（2026-10-10）

方法：`anchor idl fetch` 拉取各 DEX 程序链上现役 IDL（Whirlpool/Pancake/Meteora DLMM/PumpSwap/Byreal），
Raydium v4/CPMM/CLMM 无链上 IDL 账户，改用 `uxuyprotocol/solanaswap` 生成包（`defi/*/idl.json`、`discriminators.go`、`instructions.go`，由真实主网交易驱动维护）作对照源；
逐指令比对三要素：discriminator/selector、参数编码（字节数与字段序）、CPI 账户序。

| Dex 变体 | 目标指令 | selector/参数 | 账户序 | 结论 |
|---|---|---|---|---|
| RaydiumSwap | AMM v4 `swapBaseIn`(tag 9) | ✅ 18B 数据 | ✅ 18 账户 | 一致 |
| RaydiumCpmmSwap | `swap_base_input` | ✅ disc `8fbe5adac41e33de` 与 solanaswap 一致，2 args | ✅ 13 账户 | 一致 |
| RaydiumClmmSwap | `swap` | ✅ 41B = 4 args（u64,u64,u128,bool） | ✅ 10 核心 + bitmap/3 ticks 尾部 | 一致 |
| RaydiumClmmSwapV2 | `swapV2` | ✅ 4 args，与 solanaswap builder 逐字段一致 | ✅ 13 核心 + bitmap/ticks 尾部 | 一致 |
| Whirlpool | `swap` | ✅ 42B = 5 args | ✅ 12 账户 | 一致 |
| WhirlpoolV2 | `swapV2` | ✅ 43B（含 remaining_accounts_info=None 尾字节） | ✅ 16 账户 | 一致 |
| PancakeSwapV3SwapV2 | `swap_v2` | ✅ 4 args | ✅ 13 核心 + 尾部 | 一致 |
| MeteoraDlmm | `swap` | ✅ 2 args | ✅ 15 核心 + bins 尾部 | 一致 |
| MeteoraDlmmSwap2 | `swap2` | ✅ 3 args（含空 RemainingAccountsInfo 4×0x00） | ✅ 16 核心 + bins 尾部 | 一致 |
| PumpfunammBuy3 | `buy_exact_quote_in` | ⚠️→✅ 链上已加第 3 参数 `track_volume: OptionBool`，上游快照缺这 1 字节 → **本分支已补 `0x01`** | ✅ 23 账户 | 已修复 |
| PumpfunammSell3 | `sell` | ✅ 2 args | ✅ 21 账户 | 一致 |
| ByrealClmm / ByrealClmm2 | `swap_v2` | ✅ 4 args | ✅ 13 核心 + bitmap + 6 ticks | 一致（程序另有 `swap_v3_dyn`，`swap_v2` 仍在） |

维护提醒：DEX 程序升级后此表需复核；PumpSwap 这类快速迭代的程序是重点盯防对象（本次唯一漂移即来自它）。

**Go 侧路由现状（2026-10-10，uxuyprotocol/dex @ uxuy-router）**：上表是合约 adapter 与链上 IDL 的对齐结论；Go 引擎目前仅将 Whirlpool/RaydiumCLMM/Pancake/CPMM/DLMM/RaydiumV2 六个变体路由上链——PumpSwap（链上程序现要求尾部 buyback 账户，合约 adapter 未携带）与 Byreal（动态费池拒绝 `swap_v2`，需要 `swap_v3_dyn`+稀疏 tick+pyth）虽在合约枚举中，仍走旧纯指令路径，待合约 adapter 补齐后放行。

**临时账户创建方式**：中间跳账户由 Go 侧直接用 `System.CreateAccountWithSeed`（base=用户钱包）+ `InitializeAccount3(owner=authority PDA)` 创建——`InitializeAccount3` 的 owner 只是参数、无需程序签名；合约的 `create_token_account(_with_seed)` 指令硬约束 wSOL mint（见其 `token_mint` 约束），通用 mint 走不了，两者并存各有用途。出资方（2026-10-21 起）：pump 曲线买入的 wSOL 临时账户由**用户钱包**出资（合约 after_invoke 把租金原额退给路由 payer=同一钱包，净零）；其余中间 temp 仍由 fee payer 出资（租金滞留为平台成本，靠 custody 预热消除）。**持有方（2026-10-22 起）**：hop0 为 pump buy（曲线 15/19 或 AMM 11）时 tempIn/输入 temp 的 token owner = **authority PDA**（合约关闭并退租，Go 侧不再追加尾部 close）；其余路由保持钱包持有 + 链尾 close。合约侧路由源账户约束同步放宽：owner ∈ {payer, authority PDA}（`SourceTokenAccountOwner` 运行时校验）。**Phase-2 升级（P2-1）**：hop0 pump temp 改为**可验证程序 PDA**——`PDA(["pump-hop0", payer], program)`，由合约新指令 `create_pump_hop0_temp` 创建（钱包签名并付租金，SPL owner=authority PDA，SPL/Token-2022 mint；allocate+assign 模式抗 1-lamport 抢注），swap 侧两条 pump buy 在 hop==0 时 `require!(source == 派生地址)`——共享 custody/wsol_sa 永不可能匹配派生，"custody 塞进 hop0 source" 的构造攻击向量整体关闭（hop≥1 维持 custody/temp，托管路由是那里的设计）。同 slot 同钱包并发竞争 = create 确定性失败后重试。**入金约束（2026-10-22 加固）**：create 只负责 allocate/assign/租金与 initialize（幂等判断按 SPL `state`，不按 lamports/data_len），**买入前 Go 必须把 `amount_in` 实际转入该 temp**，否则 swap 以 `InvalidSourceTokenAccount` 拒单。**proxy 入金不变量**：`source_token_account` 与 `source_token_sa` 必须是**不同**账户，且本次转入 `source_token_sa` 的差值必须 `== amount_in`（`SourceTokenSaIsSource` / `SourceTokenSaNotCredited`）——同一账户会让入金被 `from==to` 短路，等于让 DEX 花掉 PDA 名下已有余额。

## 10. PumpSwap 尾部账户与 Byreal swap_v3_dyn —— 已实现并主网验证（2026-10-13）

**PumpSwap 尾部账户（优先级高，流量大头）**。链上 pump_amm 现在拒绝不带尾部账户的 buy/sell：`[pool_v2（仅 coin_creator 已设时，readonly）] + [buyback_fee_recipient（readonly）] + [buyback_fee_recipient 的 quote-mint ATA（writable）]`（布局证据：dex 仓库 `instr/pumpswap.go` 的 `pumpTrailingAccounts` + 官方 SDK 1.20 wire）。补丁方案：`PumpfunammBuyAccounts3/SellAccounts3` 各加 3 个尾槽（`pool_v2` 未用时传 ZERO_ADDRESS，adapter 跳过零地址不追加 CPI meta——需主网 simulate 验证"多余槽位被拒/零地址可跳过"哪条成立，或改为 Go 侧按 pool 元数据精确给 2/3 个并让 ACCOUNTS_LEN 常量同步 +2/+3 两套变体）；Go 侧 `routerPrograms` 放行即恢复 PumpSwap 路由。

**Byreal `swap_v3_dyn`（按流量占比决定）**。动态费池对 `swap_v2` 报 6048 SwapDynamicFeeEnabled；`swap_v3_dyn` disc `e52ed584692828e4`，13 核心账户 + 尾部 `[bitmap(readonly), 4 个存在性探测的稀疏 tick array（writable, 按行走方向）, dyn-fee 池再追 token0/token1 pyth oracle（readonly）]`（dex 仓库 `instr/byreal.go` 全部现成）。补丁要点：变体拆 `ByrealClmm3Dyn`（尾部追加式枚举，序号 15 起），tick 段长度可变——需要 Go 侧按探测结果拼段 + adapter 按剩余账户里非零地址消费。

**已上线**：合约 `de71a19`（两网，.so 659,760B）+ dex `94a9989`。主网 simulate 三形态全绿：byreal 动态费（9GTj99…，pyth oracle 对）、byreal 标准费（87pbGHxi… wSOL/Bybit 深池，原生 SOL 入含 wrap+sync）、pump buy（12x41vhk…，尾部三件套、pool_v2 占位跳过）。实现要点：段与适配器槽位一一对应（适配器复用段槽 [0] 作为 CPI 程序 ID meta，段核心 19 槽，多插一个程序 ID 槽会整体错位）；buy 的 `global_volume_accumulator` 必须 WRITABLE（适配器 CPI 要求，legacy 直调时 readonly 能过是因为权限检查发生在 CPI 边界）；creator-set 泵池当前主网 40/40 全为 T2022 base，被 MintsAllSPL 闸门排除，pool_v2 实槽路径按构造覆盖。
