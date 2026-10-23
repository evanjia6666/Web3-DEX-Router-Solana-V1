# devnet probes：路由层负向用例（无需 DEX 流动性）

对**已部署到 devnet** 的 `uxuy-router` 发探针交易（`simulateTransaction`，不改状态），
断言命中预期错误码。覆盖那些"不需要真实池子"的不变量。

| 脚本 | 用例 | 期望 |
|---|---|---|
| `probe-guards.js` | proxy：`source_token_account == source_token_sa` | `SourceTokenSaIsSource` |
| | `routes = [[ ]]` | `EmptyRoute` |
| | `swap_tob_v3` `trim_rate>0` 且无尾随账户 | `InvalidAccountsLength` |
| `probe-hop0.js` | `create_pump_hop0_temp`（真实上链） | temp 165B、state=1、mint/owner 正确 |
| | hop0 pump buy：temp 未转入 `amount_in` | `InvalidSourceTokenAccount` |
| | hop0 pump buy：custody 当源 | `InvalidHopAccounts` |
| `probe-meteora.js` | MeteoraDlmmSwap2 19 槽窗口（真实 lb_pair + 真实位图扩展） | 窗口解析通过、CPI 到达 meteora，停在 `reserve_x` 3012 |

## 跑法

```bash
cd scripts/devnet-probes
npm install
node probe-guards.js     # 期望 3/3 passed
node probe-hop0.js       # 期望 create:1/1, guards:2/2
node probe-meteora.js    # 期望 PASS（stop code 3012）
```

- 需要 `target/deploy/uxuy_devnet_payer-keypair.json`（devnet 升级权限 + 付费），
  或用 `PAYER_KEYPAIR=/path/to/keypair.json` 覆盖。
- RPC 默认 `api.devnet.solana.com`（公共端点限流严重），可用
  `RPC_URL=https://solana-devnet.g.alchemy.com/v2/<key>` 覆盖（dex 项目 `.env` 的
  Alchemy key 换 `solana-devnet` 域名即可用）。部署同理：`solana program deploy --url <该地址>`。
- 首次运行会创建 1 个测试 mint + 3 个 token 账户 + hop0 temp（约 0.01 SOL devnet 租金），
  密钥缓存在 `keys.json`（已 gitignore）；后续运行复用。
- 脚本里的 `PROGRAM_ID` / `AUTHORITY_PDA` 是常量，换程序地址需同步修改
  （`probe-meteora.js` 支持 `PROGRAM_ID=` 覆盖）。

## probe-meteora.js 能证什么、不能证什么

**能证**：swap2 窗口的账户顺序/类型、路由编码（`Dex::MeteoraDlmmSwap2 = 10`）、
以及部署字节上我们的 CPI 确实进到 meteora（用真实 devnet lb_pair +
真实 `["bitmap", lb_pair]` 扩展 PDA，src/dst 故意用错 mint，于是 meteora 在
扩展槽之后才失败）。

**不能证**：位图扩展槽的**可写标志**。devnet 的 meteora 程序（2,198,048 B /
`d8fa183b…`）与主网（2,229,776 B / `d296c677…`）不是同一构建，devnet 版对该槽
**没有** `mut` 约束：把扩展槽在交易里或 CPI 里传成只读，新旧两版都照样过
（2026-10-23 用 pre-fix 字节在临时 program id 上做过 A/B，结果相同）。
`ConstraintMut(2000)` 只在主网 Kimchi/SOL 池 `ENpXvFPb…` 上复现，所以那条修复
只能在主网窗口用 gdex 侧探针闭环。

## 已验证记录

- 2026-10-22，devnet 合并版（`e55bbf65…`，617,296 B）：6/6 通过；
  `create_pump_hop0_temp` tx
  `5GCenyf2c1t6mn21XxAgNmDXqGZQGgDDrDCaiy7CGLFUyoRqVf49VKBdV1j1S5NYrNvNBbwj4SrEvWpQvPArxWpB`。
- 2026-10-23，devnet 升级到 `da5215e1…`（meteora 位图扩展可写修复，617,296 B，
  升级 tx `TA6E2qcPbEknQiByr6mGrGSzawvjD9knPPEpwrxCS8GUuJxxoggcEyydt3qLsBegGMjogVSig1AohS7yfpwDB3L`）：
  `probe-guards` 3/3、`probe-hop0` create 1/1 + guards 2/2、`probe-meteora` PASS。
