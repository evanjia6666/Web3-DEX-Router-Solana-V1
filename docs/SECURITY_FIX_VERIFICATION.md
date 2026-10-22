# 2026-10-22 安全修复：复现与验收手册

本文对应 `uxuy-router` 2026-10-22 的修复批次（proxy 入金不变量、pump hop0 入资校验、
ATA 三 seed 派生、v2 token 差额转发、`create_pump_hop0_temp` 幂等、若干加固）。
每条都给 **改造前的攻击/失败形状**、**改造后的预期错误码** 和 **链上回归断言**。

> 复现需要 Go 侧按 adapter 窗口表构造 `remaining_accounts`（见
> `docs/UXUY_ROUTER_INTEGRATION.md` 的 wire 表）。窗口填错会先在 `array_ref`
> 或 hop 校验处失败，看不到本文预期的错误码——那是构造问题，不是修复失效。

## 0. 修复一览

| # | 问题 | 修复位置 | 新增错误码 | 关键断言 |
|---|------|----------|-----------|----------|
| P0-1 | `source_token_account == source_token_sa` 时入金被 `from==to` 短路，DEX 花掉 PDA 存量 | `processor/proxy_swap_processor.rs` | `SourceTokenSaIsSource` / `SourceTokenSaNotCredited` | 入金后 `source_token_sa.amount` 必须正好 `+amount_in` |
| P0-2 | hop0 pump 买入不校验 temp 入资，花 authority 原生 SOL | `adapters/pumpfun.rs` `buy_with_layout` | `InvalidSourceTokenAccount` | `temp.lamports() >= amount_in` |
| P1-1 | ATA 派生两 seed（少 `token_program`），派生地址永不可用 | `adapters/pumpfun.rs`、`adapters/pumpfunamm.rs`、`utils/token.rs` | — | `associated_token_address(authority, WSOL, Token)` == `wsol_sa::ID` |
| P1-2 | `create_pump_hop0_temp` 的「已初始化」判断恒真，`initialize_account3` 是死代码 | `instructions/create_pump_hop0_temp.rs` | — | create 后 temp 的 SPL `state == 1` 且 `mint/owner` 正确 |
| P1-3 | v2 hop0-final 用 lamports 差值转发（token 移动不改 lamports，恒 0） | `adapters/pumpfun.rs` `PumpfunV2Processor` | — | 转发量 == `amount` 差值；buy 側整余额、sell 侧只转增量 |
| P1-4 | 空 hops 成功、trim 裸索引 panic、hop0 不走 SA 白名单、claim 扫穿水位、ToB 的既有 WSOL 滞留 | `common_swap.rs`、`swap_v3.rs`、`adapters/common.rs`、`claim.rs`、`swap_tob_processor.rs`、`constants.rs` | `EmptyRoute` | 见第 3 节 |

## 1. 复现 A：proxy 零成本花掉 PDA 余额（P0-1）

**前置**：authority PDA（`5rvVfdVCgSQ9AyTTctN1MPGk5brWVEtvMfPKtcesAuVG`）名下任意一个
非空 SPL 代币账户 `V`——`wsol_sa`（`G6KLoMzx47Nh1xJLtzyGiFXiwH688Z8LEwHtFv4xBPyj`）、
custody 中间账户、或预建的 PDA temp 都满足。

**攻击交易（改造前可成功）**

- 指令：`proxy_swap`（`swap_v3*` 同通道，费率填 0）
- 账户：
  - `payer` = 攻击者（签名）
  - `source_token_account` = `V`
  - `source_token_sa` = `V`（**同址**）
  - `sa_authority` = authority PDA
  - `source_mint` = `V.mint`，`destination_token_account` = 攻击者的输出币账户
  - `remaining_accounts` = 某个能 `V.mint → 输出币` 的 DEX 窗口（源槽填 `V`、authority 槽填 PDA）
- args：`amount_in = V.amount`，`expect_amount_out = 0`，`slippage = 0`
  → `min_return = 0`；`post_swap_check` 看到源差值正好 `amount_in`，全部通过。

改造前结果：`V` 的余额被换成攻击者账户里的输出币，攻击者零出资。

**改造后预期**

- 先撞 `SourceTokenSaIsSource`（同址），不再进入 DEX CPI；
- 把 `source_token_sa` 换成另一个 PDA 账户但**不做真实入金**：撞 `SourceTokenSaNotCredited`
  （实测入金差值 `!= amount_in`）。

**链上回归断言**（建议纳入 QA 冒烟）

1. 任意用户交易前后，`wsol_sa` 的 token `amount` 不得减少（除平台自己的 `claim`）。
2. authority PDA 的 lamports 不得因用户路由减少（pump 首建租的既有设计除外，见下）。

## 2. 复现 B：空 temp 花 authority 原生 SOL（P0-2）

**攻击交易（改造前可成功）**

1. `create_pump_hop0_temp(payer = 攻击者, token_mint = 任意 SPL mint)`。
   改造前这条指令因早退**从不**调用 `initialize_account3`，留下的就是「已 allocate/assign、
   未初始化」的 165B 账户；攻击者再补一条 SPL `InitializeAccount3`（`owner` 参数填 authority PDA，
   无需签名）即可得到一个合法的空 token account。
2. `swap`（`swap_v3` 或 `v3_with_cpi_event` 均可，费率为 0）：
   - `source_token_account` = 上一步的 temp（owner = authority PDA，余额 0）
   - `remaining_accounts` = `PumpfunBuy202610`（或 `PumpfunBuy`）的 20 槽窗口，
     `swap_authority_pubkey` = PDA，`swap_destination_token` = 攻击者可收到的账户（必要时先落 PDA ATA 再走第二跳）
   - args：`amount_in` = authority 余额的绝大部分，`expect_amount_out = 1`，`slippage = 0`
3. `before_invoke` 把 temp close 进 authority（只带回 2,039,280 租金），pump 的
   `buy_exact_sol_in` 从 user 槽（PDA）扣 `amount_in`；temp 已 close，
   `post_swap_check` 的源差值校验整段跳过。

改造前结果：authority 净损失 ≈ `amount_in − 2,039,280`，可重复。

**改造后预期**

- 空 temp（或任何 `lamports < amount_in` 的 temp）在 CPI 之前撞 `InvalidSourceTokenAccount`；
- 正常路由（先转入 `rent + amount_in`）不受影响；`create_pump_hop0_temp` 现在会真正初始化，
  同 slot 重试按 SPL `state` 判定幂等。

## 3. 其他修复的快速验证点

| 修复 | 验证方式 | 预期 |
|------|----------|------|
| ATA 三 seed | `cargo test -p uxuy-router test_associated_token_address_matches_wsol_sa` | 通过（派生 == `wsol_sa`） |
| token amount 偏移 | `cargo test -p uxuy-router test_token_account_amount_reads_spl_layout` | 通过（`amount` 在 64..72） |
| AMM buy 可用性 | 任意 `PumpfunammBuy3` 路由（sink = `ATA(uva, quote_mint, quote_program)`） | 不再 `InvalidHopAccounts` |
| 曲线 v2 buy hop0 | `PumpfunBuyV2` 首跳，source = canonical quote ATA（三 seed） | 通过 ATA 校验 |
| v2 末跳转发 | USDC-quote 的 `PumpfunSellV2` hop0-final | 用户边账户收到 quote，不再 0 转发 |
| 空路由 | `routes = [[ ]]` | `EmptyRoute` |
| trim 短账户 | `swap_v3(trim_rate > 0)` 且无 trailing 账户 | `InvalidAccountsLength`（不再 panic） |
| hop0 白名单 | hop0 窗口夹带额外 PDA-owned 代币账户 | `UnexpectedSaTokenAccount` |
| claim 水位 | authority 余额 < 6M 时 `claim` | `InsufficientFunds` |
| ToB 既有 WSOL | 用户 destination 先存 WSOL 再走 SOL 手续费路径 | 既有部分随 close 一起退回 payer，不再滞留 PDA |

## 4. 门禁

```bash
cargo test -p uxuy-router        # 19 passed
bash scripts/size-check.sh       # 818,248 B（ceiling 1,017,728）
```

## 5. 仍未覆盖（已在集成手册标注）

- **v2 buy 的 canonical quote ATA 存量**：顶层转入无法与历史存量区分。推荐 Go 改走
  `source_token_sa` 入金（合约会校验差值），或保证 canonical ATA 每笔前后归零。
- 6 个 adapter 的 `token_program / memo_program` 槽未钉死：属纵深防御，利用链取决于目标 DEX 是否自钉。
- 费率由调用方声明（`commission_info` / `platform_fee_rate`）：协议级设计，非本批修复范围。
