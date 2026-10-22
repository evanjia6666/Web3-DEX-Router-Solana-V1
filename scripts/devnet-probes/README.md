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

## 跑法

```bash
cd scripts/devnet-probes
npm install
node probe-guards.js     # 期望 3/3 passed
node probe-hop0.js       # 期望 create:1/1, guards:2/2
```

- 需要 `target/deploy/uxuy_devnet_payer-keypair.json`（devnet 升级权限 + 付费），
  或用 `PAYER_KEYPAIR=/path/to/keypair.json` 覆盖。
- 首次运行会创建 1 个测试 mint + 3 个 token 账户 + hop0 temp（约 0.01 SOL devnet 租金），
  密钥缓存在 `keys.json`（已 gitignore）；后续运行复用。
- 脚本里的 `PROGRAM_ID` / `AUTHORITY_PDA` 是常量，换程序地址需同步修改。

## 已验证记录

2026-10-22，对 devnet 合并版（`e55bbf65…`，617,296 B）全部 6 条通过；
`create_pump_hop0_temp` tx `5GCenyf2c1t6mn21XxAgNmDXqGZQGgDDrDCaiy7CGLFUyoRqVf49VKBdV1j1S5NYrNvNBbwj4SrEvWpQvPArxWpB`。
