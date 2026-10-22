# devnet 验收清单：phase-2 合并版（`uxuy-router` @ `b37992b`）

## 0. 当前状态

- devnet 程序已是合并版：**617,296 B / `e55bbf65…47a1`**，链上字节校验 **MATCH**，
  slot 513,850,011（sig `3hrPnLfV21EL…UTNE`）。
- devnet 回滚件：`target/deploy/rollback/devnet-live-848120.so`（`220912f8…`）——
  升级前从链上 dump 的原始 ELF，可直接原样退回。
- 主网仍是 `71ee5a9` 基线（校验 MATCH），未升级。

> 每条用例都请记录：tx 签名、`unitsConsumed`、涉及的 token/native 余额前后值。
> 余额断言比"交易成功"更重要——本项目的历史事故都发生在"成功但结算错"。

## 1. 前置（Go 侧）

1. wire 切到新形状：AMM buy/sell 27 槽、v2 buy 29 / sell 28（含 user-edge 尾槽）、
   hop0 pump 走 `create_pump_hop0_temp` + 实际入金。
2. proxy 路由：`source_token_account` 与 `source_token_sa` 必须是**不同**账户。
3. 准备三个身份：普通用户、proxy 路由、authority PDA（余额 ≥ 6M，供首建租闸门）。

## 2. 正向用例

| # | 路由 | 关键断言 |
|---|---|---|
| 1 | pump 曲线 buy，hop0（Dex 15/19 或 202610 变体） | `create_pump_hop0_temp` → 转入 `rent+amount_in` → 成交；temp 被关闭、租金原额退回 payer；authority lamports 只因首建租下降 |
| 2 | pump 曲线 sell，hop0 | 成交；quote 到用户边账户；无 temp 残留 |
| 3 | pump 曲线 buy/sell，hop≥1 | custody 作中间源，正常结算 |
| 4 | PumpSwap `buy3`（Dex 11） | 成交；第 4 尾槽 = `ATA(uva, quote_mint, quote_program)`；若该币有 cashback，余额落在 sink |
| 5 | PumpSwap `sell3`（Dex 12） | 成交；quote 到用户边账户 |
| 6 | pump v2 buy（Dex 17），hop0-final | base 从 temp 整额转发到用户边账户、temp 关闭；canonical quote ATA 实扣 `amount_in` |
| 7 | pump v2 sell（Dex 18），hop0-final | quote 从 canonical ATA 只转发本跳增量；源 temp 关闭 |
| 8 | proxy 正常路由（非 pump） | 入金后 `source_token_sa.amount` 增量 == `amount_in`；结算与 min_return 正常 |
| 9 | 3 跳组合（含 pump hop0 + 一个 AMM hop） | 逐跳结算正确、无余额滞留（**这条同时用于 CU 测量**） |
| 10 | `claim` token 分支 | 能从 canonical quote ATA / custody 扫出指定 mint 的全额 |

## 2.5 已完成的路由层验证（2026-10-22，打在 devnet 合并版 `e55bbf65…` 上）

`scripts/devnet-probes/` 下的探针已跑通 **6/6**，不需要 DEX 流动性：

| 用例 | 结果 | 证据 |
|---|---|---|
| proxy 同址 | ✅ `SourceTokenSaIsSource` | `probe-guards.js` |
| 空 hop 列表 | ✅ `EmptyRoute` | `probe-guards.js` |
| trim 短账户 | ✅ `InvalidAccountsLength` | `probe-guards.js` |
| `create_pump_hop0_temp` | ✅ temp 165 B / state=1 / mint+owner 正确 | tx `5GCenyf2c1t6…WpB` |
| hop0 空 temp | ✅ `InvalidSourceTokenAccount` | `probe-hop0.js` |
| hop0 custody 当源 | ✅ `InvalidHopAccounts` | `probe-hop0.js` |

> 这 6 条都是在**部署后的字节**上验证的（链上 sha256 与本地产物 MATCH），
> 不是单元测试。剩余的是需要流动性/Go 侧构交易的正向路径。

## 3. 负向用例（必须精确命中错误码）

| # | 构造 | 期望错误 |
|---|---|---|
| 1 | proxy：`source_token_account == source_token_sa` | `SourceTokenSaIsSource` ✅ 已验证 |
| 2 | proxy：`source_token_sa` 未实际入金（差额 != amount_in） | `SourceTokenSaNotCredited` |
| 3 | `routes = [[ ]]`（空 hop 列表） | `EmptyRoute` ✅ 已验证 |
| 4 | `swap_v3` 带 `trim_rate > 0` 但无尾部账户 | `InvalidAccountsLength`（不再是 panic）✅ 已验证 |
| 5 | hop0 pump buy：temp 只带租金、未转入 `amount_in` | `InvalidSourceTokenAccount` ✅ 已验证 |
| 6 | hop0 pump buy：把 custody / `wsol_sa` 当源 | `InvalidHopAccounts`（派生校验）✅ 已验证 |

## 4. CU 检查

对**用例 9**做 `simulateTransaction`，取 `unitsConsumed`：

- 硬门槛：`unitsConsumed` ≤ 调用方设置的 compute limit × 0.9。
- 回归门槛：与 opt-s 历史值相比涨幅 ≤ 5%。本项目 `opt-level` 从 `s` 改成了 `z`，
  size-optimised codegen 有可能多花几个百分点 CU。
- 若超标：改用 **opt-s 合并版** `target/deploy/rollback/uxuy-router-opts-677416.so`
  （677,416 B / `cf47d32f…`，buffer rent **4.716 SOL**）。
  ⚠️ 它比现有余额 4.646 SOL 多 **0.07 SOL**，切回前要补一点。

## 5. 判定

- 10 条正向全过 + 6 条负向全中 + CU 双门槛达标 → 进主网窗口（与 gdex 同窗，先合约后 gdex）。
- 任一失败 → 停留 devnet，按上面回滚件恢复不是必须的（可继续修）；**不要**带着失败项上主网。

## 6. 产物与哈希

| 用途 | 文件 | 字节 | sha256 |
|---|---|---|---|
| devnet 当前 / 主网候选（opt-z） | `uxuy-router-optz-617296.so` | 617,296 | `e55bbf657f78c35e05cc4a73237563aa10f14dfc467d5b437f2dbcaea62747a1` |
| CU 回退候选（opt-s） | `uxuy-router-opts-677416.so` | 677,416 | `cf47d32f6d14256f1b9fd3391a35544e99e8bd158d2d6f299f2222ed9ad05d65` |
| devnet 回滚 | `devnet-live-848120.so` | 848,120 | `220912f845cb80fb0f3ce8b2efa228eddb7ac23fbd1b295847d167041adb2dfb` |
| 主网回滚基线 | `71ee5a9-live-652304.so` | 652,304 | `685fb20bf69406f9d63e5dcfcb353441849f75b338677114402b1c86eb365ec4` |
