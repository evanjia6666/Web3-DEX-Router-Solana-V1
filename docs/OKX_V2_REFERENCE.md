# OKX Router V2（proVF4…）链上实现参考

> 数据来源：V2 未开源且**链上 IDL 已关闭**（anchor:idl PDA 不存在），本文全部结论来自
> 2026-10-13 对 17 笔主网真实交易 + ALT 账户 + 托管账户状态的逆向。样本签名存于
> 会话记录；代表样本 `5JmRdq3P…`（5-hop/950B）、`CT9z9o2t…`（1-hop/428B）。

## 1. 总体架构（与 V1/我们的 fork 对照）

| 维度 | OKX V1（我们的 fork） | OKX V2（实测） |
|---|---|---|
| 消息版本 | legacy v1 | **v0 + Address Lookup Table** |
| 交易尺寸 | 1-hop ≈950B；2-hop receiver 1521B（超限） | **428–1117B，含 4 路并行 + 5 跳** |
| 中间账户 | 每笔 CreateAccountWithSeed + Init3，owner=authority PDA，**不关（租金滞留）** | **全局托管账户**（见 §3），租一次、永不关 |
| 路由模型 | routes: Vec<Vec<Route>>（合约支持，Go 只发单路径） | **两级路由真实在用**：实测 4 路并行（amount_in 按 weight 拆分：52840+55482+103038+52842=264202） |
| 输入语义 | args.amount_in 指定 | **余额差/TokenLedger**：`amount_in` 可等于账户既有余额；携带 `Sysvar1nstructions`（读前置指令实际产出） |
| SwapArgs | {order_id, amount_in, expect_amount_out, slippage u16 bps, routes}（我们已对齐） | 相同形态（日志实证：`expect_amount_out: 1, slippage: 0` 与 `slippage: 200`） |
| 佣金 | commission_info 单值 bps | **commission_rates: Vec**（多佣金账户）+ trim + charge |
| 原生 SOL 输出 | acc_close_flag + sol_receiver 尾部 | **`output_native_sol` 指令内标志** |
| 指令清单 | swap_toc/tob/tob_enhanced/v3/with_receiver、create_ata_with_close_authority、wrap_unwrap… | 日志实证：**SwapTob、SwapTobV3、SwapTobEnhanced、SwapToc、CreateTokenAccount** |
| DEX 枚举 | 15 个（Raydium×4/CPMM/Whirlpool×2/Pancake×2/DLMM×2/Pump×2/Byreal×2） | 实测 18+：另含 **GoonFi、Manifest、ZerofiWithSig、BisonfiWithSig、Aquifer、Deriverse、MeteoraDbc、MeteoraDAMMV2、TesseraV2、TaurusFi2、ByrealClmm3、Pumpfun(旧)** |
| ProgramID 填充 | 惰性槽传程序自身 ID（=None） | **相同技巧**（proVF4 自身填充可选槽） |

## 2. ALT 分层（尺寸问题的答案）

- **全局热表** `GP57a1T8vJjeAnF2zfKT1bnZysNwVprLaw6Mjw2d1gPy`（≈133 地址，9/17 笔引用）：
  `[0]` 自家程序；`[1..~60]` 各 DEX 程序；`[60..63]` wSOL/USDC/USDT/USD1 mint；
  `[64..69]` Token2022/SPL Token/ATA/System/ALT 程序/ComputeBudget；
  `[107..109]` Memo/Sysvar-ix/Rent；`[110..]` 事件权限、Jito tip front（`jitodontfront…`）、
  优先费池（`pfeeUxB…`）等；**authority PDA + 三个全局托管账户也在表内**。
- **按路由的池子表**（约 40 张，每张引用 1-2 次）：池、vault、tick array 等一次性账户。
- 效果：5-hop/101 账户/4 路并行的交易 = **950 字节**（12 静态 key + 35w/12r ALT 引用）。

## 3. 全局托管账户模型（租金与中间账户的答案）

实测所有权（getMultipleAccounts）：

| 账户 | mint | owner | 余额（样本时刻） |
|---|---|---|---|
| `ARu4n5mF…` | —（PDA，系统程序拥有，1.25 SOL） | authority PDA | — |
| `4CqpDzLQ…` | USDT | authority PDA | 344,873,957 |
| `DT2krA8v…` | wSOL | authority PDA | 6,333,712,325（lamports 6,335,200,765，含未 sync 原生进账） |
| `8XrtGP8R…` | USDC | authority PDA | 1,890,022,965 |

资金流（5-hop 样本，USDT→USDC）：

1. 声明 source/destination = **用户自己的 ATA**（owner=用户，实测 EH6q9neh=USDT ATA、6vV9YppG=USDC ATA）；
2. 交易内先从用户 ATA 划 `amount_in` 进 USDT 托管账户；
3. 4 条并行路径在托管账户之间完成兑换（每跳 `amount_in` = 前跳实际产出，合约内链式）；
4. 末尾从 USDC 托管账户划实际产出至用户 USDC ATA；
5. 全程对声明 source/dest 做前后余额差校验（`source_token_change`/`destination_token_change` 日志）。

**后果**：托管账户租金只付一次（wSOL 托管账户 lamports>amount，原生进账暂存其中）；
多跳无需任何临时账户创建/关闭；余额差记账是必须的（全局账户余额跨用户共享，不可作
绝对量）。代价：路由器成为事实上的资金托管方（巨额沉淀 + 安全边界），需配套
审计/多签/风险准备金。

## 4. 我们 fork 的采纳路线（2026-10-13 修订：Transaction v1 取代 ALT）

> **修订原因**：SIMD-0385「Transaction v1（更大交易）」已于 **2026-09-15** 随
> Agave 4.2.2 在主网激活：包上限 **1232 → 4096 字节**（base64 提交通道），内联
> compute 配置，**不支持也不需要 ALT**（64 个内联地址 ≈2KB 已富余；SIMD-0596 草案
> 提至 96）。我们的 2-hop receiver 链序列化后 1499 字节——**v1 直接放下，无需任何
> ALT 基础设施**（实测：公共主网 RPC simulate 绿灯 + 资金流断言通过，见 dex 仓
> e2e TXV1 模式，commit 309456a）。OKX 抽样仍全用 v0+ALT 是其钱包兼容选择。
> ALT 降级为「消费端不支持 v1 时」的兼容备选。

### 第一步（已验证）：超大链改发 Transaction v1 — 解决尺寸上限
- v1 要点：签名移到尾部、版本字节在首位（0x81）；compute 预算（CU limit、优先费
  总额、loaded-accounts-data、heap）移入消息头配置位；**ComputeBudget 指令变为
  空操作**（仍烧 150 CU/条，v1 下不要再发）；账户列表去重要求更严格。
- 两个实测坑：① v1 的 loaded-accounts-data 默认上限低于 legacy，必须显式
  `WithLoadedAccountsDataSizeLimit(64MiB)`，否则 `MaxLoadedAccountsDataSizeExceeded`；
  ② 提交须 base64 编码（base58 通道仍限 1232）。
- Go：solana-go v1.23.0 原生支持（`TransactionV1Config`）；dex 侧在服务端组装/
  simulate/unsignedTransaction 输出处按「legacy 超限时切 v1（或全量 v1）」接入，
  响应的 `transactionVersion` 字段相应置位；提交端需 `maxSupportedTransactionVersion`。

### 第二步：中间跳改全局托管账户 — 已实现并主网验证（dex `4fe9955`），零合约改动
- 落地形态：托管账户 = `CreateWithSeed(ops钱包, "uxuysa-"+sha256(mint)[:8], SPL Token)`，
  SPL owner = authority PDA（InitializeAccount3 指定，无需签名）。**合约完全不用改**：
  现行中间账户本就是 authority PDA 持有，`before_check` 的 PDA authority 校验天然满足，
  余额差记账对既有余额成立，逐笔净流量为零（进出精确金额）。
- Go：`router.SetCustodyAccount(mint, addr)` 启动时注入热表；compose 命中即用托管
  账户（省两条建户指令），冷门 mint 自动回退现行每笔临时账户。预热工具：
  `e2e -mode warm-custody`（ops 签名，一次性 ~0.002 SOL/mint）。
- 主网实证（公共 RPC，v1 交易）：USDT 托管 `9ZMzDQZt…`（预热 sig `5GWC3V5w…`）后，
  2-hop receiver 链 6→4 条指令 / 1345 字节，**A 净成本从 −2,044,280（中间租金+费）
  降为 −5,000（纯手续费）**；receiver 恰得产出不变。
- 与 V2 差异：我们的托管地址派生自 ops 钱包而非程序 PDA（省一次合约升级）；
  资金语义相同（共享通道账户、净流量零、不关闭）。

### 第三步：能力对齐（可选，按业务需要；返佣明确不做）
- ~~`commission_rates: Vec`~~（**不做**，业务决策 2026-10-13）；
  `output_native_sol` 指令内标志；
  TokenLedger（`Sysvar1nstructions` 读前置指令产出，支持多 router 指令组合）；
  DEX 扩容（GoonFi/Manifest/… 按 traffic 排序）。
- 我们的 V1 合约骨架（两级路由、余额差、SwapArgs V2 形态）与 V2 同构，
  上述均为增量。

## 5. 与当前状态的映射

- 当前：合约 `c80268b`（主网+devnet），dex `8c646f3`；1-hop receiver 已主网
  simulate 绿灯（receiver 恰得产出 / A 仅付手续费）；**2-hop receiver 已以 v1
  交易在公共主网 RPC simulate 绿灯（1499B）**。
- 第一步（v1）已 e2e 验证，剩服务端接入；第二步替代「合约逐跳关户」方案
  （docs/UXUY_ROUTER_INTEGRATION.md 的待办），且顺带消灭每笔租金垫付/返还的
  交易内往返。
- 第三步不阻塞任何现有功能。
