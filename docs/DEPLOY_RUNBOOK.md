# 热修部署手册：proxy 入金不变量（`fafecf4`）

> **分支选择（2026-10-22 合并后）**
> - `uxuy-router`（当前分支）= phase-2 + 本热修 + 加固。部署它即 **phase-2 发布**，
>   含 3 处 wire 变更（AMM 27 槽、v2 29/28 窗口、`create_pump_hop0_temp`），
>   **必须与 gdex 同窗口**。
> - `hotfix/proxy-deposit` / `size/opt-z` = 仅 P0-1，可单独升级。
>
> 部署件大小与 buffer rent：hotfix **598,104 B / 4.164 SOL**；
> 合并版 **617,296 B / 4.297 SOL**（付费账户现有 4.646 SOL，均放得下）。
>
> **已选路线 B（phase-2 全量）。** devnet 已于 2026-10-22 升级到合并版并完成
> 字节校验（见 §8）；主网待 gdex 同窗口。

## 0. 结论先行

- 线上程序已用**字节比对**确认 = `71ee5a9`（2026-10-22 13:44 本机构建、13:45 部署）。
- 本热修 = `71ee5a9` + 单提交 `fafecf4`（P0-1：`SourceTokenSaIsSource` /
  `SourceTokenSaNotCredited`），**不携带任何 phase-2 内容**。
- 因此**可以单独升级，不需要 gdex 同窗口**（`97f5be0` 的 pump PDA-user 统一已在线上）。

## 1. 产物

| 用途 | 文件 | 字节 | sha256 |
|---|---|---|---|
| 线上基线（回滚用） | `target/deploy/rollback/71ee5a9-live-652304.so` | 652,304 | `685fb20bf69406f9d63e5dcfcb353441849f75b338677114402b1c86eb365ec4` |
| devnet 原字节（devnet 回滚用） | `target/deploy/rollback/devnet-live-848120.so` | 848,120 | `220912f845cb80fb0f3ce8b2efa228eddb7ac23fbd1b295847d167041adb2dfb` |
| 热修部署件（opt-s，第一版） | `target/deploy/rollback/fafecf4-hotfix-654824.so` | 654,824 | `61be408bc66a5227a723825b397f2effb4f3e61f5c109efffb207210bd2fa26a` |
| **热修部署件（opt-z，推荐）** | `target/deploy/rollback/optz-hotfix-598104.so` | **598,104** | `aa0cbee7eaed661bdf4a545b85a4d16a83adf7e4de5fa40be4212b13c112dd66` |
| **合并版部署件（phase-2 + 修复，opt-z）** | `target/deploy/rollback/uxuy-router-optz-617296.so` | **617,296** | `e55bbf657f78c35e05cc4a73237563aa10f14dfc467d5b437f2dbcaea62747a1` |
| CU 回退候选（opt-s 合并版） | `target/deploy/rollback/uxuy-router-opts-677416.so` | 677,416 | `cf47d32f6d14256f1b9fd3391a35544e99e8bd158d2d6f299f2222ed9ad05d65` |
| devnet 现状（**无法作为回滚件**，见 §5） | `target/deploy/rollback/devnet-prev-848120.so` | 848,120 | `4e2cef99…`（与 devnet 链上 UNKNOWN 不匹配） |

构建配方（必须 v1.58、sbpfv2、strip；sbpfv3 会被集群拒收）：

```bash
source ~/.config/uxuy/sbf-env.sh
cargo build -p uxuy-router --release --target sbpfv2-solana-solana
~/.cache/solana/platform-tools-v1.58/rust/lib/rustlib/aarch64-apple-darwin/bin/llvm-strip \
  --strip-all target/sbpfv2-solana-solana/release/uxuy_router.so -o <stripped.so>
```

**可复现性已验**：在 `fafecf4` 上重建 = `61be408b…`（与本表一致）。文档/脚本提交不影响 `.so`。

> `target/deploy/rollback/` 在本机 `target/` 下（gitignored），`cargo clean` 会清空。
> 部署前把两个 `.so` + `SHA256SUMS.txt` 拷进密钥管理系统或受控存储。

## 2. 部署前检查（逐条打勾）

1. **升级权限**：`solana-keygen pubkey target/deploy/uxuy_upgrade_authority-keypair.json`
   == `4jZq7kXH6XA6xCZvnAvL9NPxN4QFDNHdKfw1zDm1hMwg`（主网 chain 上的 authority）。
2. **确认线上仍是基线**：
   `bash scripts/verify-onchain-elf.sh target/deploy/rollback/71ee5a9-live-652304.so` → `MATCH`
3. **差异面**：`git diff --stat 71ee5a9..fafecf4` = 3 files, +70/−1。
4. **账户容量**：线上 ProgramData 834,792 B（ELF 652,304 + 182,488 零填充）
   > 热修 654,824 B，**无需 realloc / `--max-len`**。
5. **付费账户余额**：`solana program deploy` 会先建 buffer，需按 .so 大小垫付
   rent-exempt（`(size+128)*6960` lamports，升级完成后退回）：
   - opt-s 654,824 B → 4.558 SOL；opt-z 598,104 B → 4.164 SOL；
     合并版 617,296 B → **4.297 SOL**。
   - `uxuy_ops_payer`（`HRF4WPP5…`）当前 **4.646 SOL**：合并版余 0.35 SOL，热修版余 0.48 SOL。
   - 付费账户不必是 ops 钱包：`--keypair` 只是 buffer 出资方，`--upgrade-authority`
     另签；任何 ≥ 4.2 SOL 的钱包都能垫，rent 事后原路退回。
6. **Go 侧确认**：proxy 路由不会把同一个账户同时传给 `source_token_account` 与
   `source_token_sa`（否则升级后该路由会以 `SourceTokenSaIsSource` 拒单）。
7. **devnet 用例**：正常 proxy / 同址 / 未到账 三条先过（见 §4）。

## 3. 部署

```bash
# 主网
solana program deploy target/deploy/rollback/fafecf4-hotfix-654824.so \
  --program-id AN6zjJfTD5Cpj2x56sWsAod1Jb5viiQH4Xcg3tm5DgNY \
  --upgrade-authority target/deploy/uxuy_upgrade_authority-keypair.json \
  --keypair target/deploy/uxuy_ops_payer-keypair.json \
  --url mainnet-beta

# devnet（payer 同时是升级权限）
solana program deploy target/deploy/rollback/fafecf4-hotfix-654824.so \
  --program-id AN6zjJfTD5Cpj2x56sWsAod1Jb5viiQH4Xcg3tm5DgNY \
  --upgrade-authority target/deploy/uxuy_devnet_payer-keypair.json \
  --keypair target/deploy/uxuy_devnet_payer-keypair.json \
  --url devnet
```

## 4. 升级后验证

1. **字节校验**（必须 MATCH）：
   ```bash
   bash scripts/verify-onchain-elf.sh target/deploy/rollback/fafecf4-hotfix-654824.so
   bash scripts/verify-onchain-elf.sh target/deploy/rollback/fafecf4-hotfix-654824.so https://api.devnet.solana.com
   ```
2. `solana program show`：Last Deployed Slot 前进、Authority 不变。
3. authority PDA `5rvVfd…` 的 lamports 不因升级变化（升级费走付费账户）。
4. `wsol_sa`（`G6KLo…`）仍不存在 / 余额不变；PDA 名下代币账户不变。
5. 业务冒烟：一条 pump 路由 + 一条 proxy 路由。
6. 负向用例（devnet，需 Go 侧构交易）：同址 → `SourceTokenSaIsSource`；
   不实入金 → `SourceTokenSaNotCredited`。

## 5. 回滚

```bash
solana program deploy target/deploy/rollback/71ee5a9-live-652304.so \
  --program-id AN6zjJfTD5Cpj2x56sWsAod1Jb5viiQH4Xcg3tm5DgNY \
  --upgrade-authority target/deploy/uxuy_upgrade_authority-keypair.json \
  --keypair <funded-payer> --url mainnet-beta
```

**devnet 回滚**：升级前已从链上 dump 原始 ELF 并归档为
`target/deploy/rollback/devnet-live-848120.so`（sha256 `220912f8…`，校验 MATCH），
devnet 可直接回滚到升级前那一份：

```bash
solana program deploy target/deploy/rollback/devnet-live-848120.so \
  --program-id AN6zjJfTD5Cpj2x56sWsAod1Jb5viiQH4Xcg3tm5DgNY \
  --upgrade-authority target/deploy/uxuy_devnet_payer-keypair.json \
  --keypair target/deploy/uxuy_devnet_payer-keypair.json --url devnet
```

## 6. 不在本热修范围

phase-2 全部内容（AMM 27 槽、v2 29/28 窗口、`create_pump_hop0_temp`、hop0 入资校验、
ATA 三 seed、v2 token 差额转发、claim 水位、ToB 退款、空路由/trim 防护）都不在这里。
它们带 3 处 wire 变更，必须与 gdex 同窗口另行发布（见 `UXUY_ROUTER_INTEGRATION.md` §7）。

## 7. 本手册的事实依据

- 线上 ProgramData `DvCC2wgb…`：834,837 B = 45 B 头 + 834,792 B 负载；
  负载前 652,304 B 与 `fafecf4^`（= `71ee5a9`）本机构建产物**逐字节相同**；
  其余 182,488 B 为零填充（历史 `--max-len` 余量，不是"未 strip"）。
- 构建可复现：`71ee5a9` 与 `fafecf4` 两棵树在本机重建均得到上表哈希。

## 8. 部署记录

| 环境 | 时间 (CST) | 版本 | 部署件 | 字节 | sha256 | 结果 |
|---|---|---|---|---|---|---|
| devnet | 2026-10-22 18:1x | `uxuy-router` @ `b37992b` | `uxuy-router-optz-617296.so` | 617,296 | `e55bbf65…47a1` | 已升级，字节校验 MATCH；slot 513,850,011；sig `3hrPnLfV21EL…UTNE`；升级前字节已归档 |
| mainnet | 2026-10-22 18:3x | 同上 | 同上 | 617,296 | `e55bbf65…47a1` | **已升级**，字节校验 MATCH；slot 460,000,401；sig `2T7TAryMLhXF…mpy` |
| mainnet | 2026-10-23 17:22 | `uxuy-router` @ `5a0b08f`（meteora 位图扩展可写） | `target/deploy/rollback/uxuy-router-20261023-da5215e1.so` | 617,296 | `da5215e1…a10e` | **已升级**，字节校验 MATCH；slot 460,387,595；sig `4JwGXLFX4dxMpnBLWTFW8ZhFJGDkWYec1HJwFeUjnWTMLC1ZWy3gjXFcfFE1c5YUpAeBjpPuwRw1bYs8bZMkuSYj`；容量仍 834,792；手续费 ~0.0068 SOL；升级后空指令探针 → `InstructionFallbackNotFound`(101)，证明新 ELF 可执行 |

> ⚠️ 主网升级必须带 `--max-len 617296`：不带时 CLI 会按 ProgramData 现有容量（834,792 B）分配 buffer，
> 需要 5.811 SOL，付费账户只有 4.646，部署会在 `Account allocation failed` 处失败（buffer 未创建、不扣款）。
> 带上后 buffer = 617,296 B（4.297 SOL），升级完成后原路退回，实际只花手续费 ~0.0068 SOL。
> ProgramData 容量保持 834,792（不回收），尾部 217,496 B 为零填充。

### gdex 侧（2026-10-22）

- 部署件：`/data/bin/gdex-solana` → `gdex_solana_vf2010e2_2026-10-22`（符号名里的
  `f2010e2` = gdex `origin/uxuy-router` HEAD，含 `create_pump_hop0_temp` 构造）。
- 服务 `gdex-master-solana.service` 于 **18:39:54 CST** 重启；启动后持续处理主网区块
  （日志 `OnSolBlockTxs` 正常），无 panic/ERROR；`/health` 返回 200。
- 合约升级（18:36 前后）与 gdex 重启（18:39:54）之间的空窗约 3 分钟。
- **尚未观测到端到端 swap**：`getSignaturesForAddress(router)` 最后一条在 18:24，
  升级后 5 分钟观察窗内零路由交易（该程序近期日均仅个位数交易）。
  真正的 wire 兼容性要等第一笔真实成交才能确认。

### 主网实盘验证：pump.fun 曲线 buy + sell（2026-10-22 19:15）

| 方向 | 交易 | slot | CU | 链上日志要点 |
|---|---|---|---|---|
| BUY 0.01 SOL → Adam | `jur4V2zrei57TnZ7piYyuZVC16i9iHB5r525wJq8UHzzyEdWY3h2z55FYJucaA4KGgtckgtEG9CgFzR2fBmqUD2` | 460,012,263 | 204,023 | `CreatePumpHop0Temp`（temp 归 authority PDA）→ `Dex::PumpfunBuy202610` → `BuyExactSolIn` → 退租 1,488,440 → 实收 107,717,925,720 |
| SELL 全部 → native | `2aE4wdKeMFta6bVEi7Fu9sz2JcBuv67MbLEwMkwTsLrbcPTpuN42gEazJgpQPANukEqbQ6EJmsE6FmP2Wc4e3tAF` | 460,012,291 | 113,157 | `Dex::PumpfunSell202610`（源=用户 ATA——曲线 sell 合约不要求 PDA temp）→ 实收 9,778,108 lamports |

钱包净变化 −0.000257 SOL（往返曲线费 ~2.2%）。两笔均 `finalized`、无错误；
预执行门限（simulate）与实际成交一致。

---

## 9. 2026-10-23 主网升级预演（未动链）

### 9.1 现状与目标

| | 值 |
|---|---|
| 程序 | `AN6zjJfTD5Cpj2x56sWsAod1Jb5viiQH4Xcg3tm5DgNY` |
| 升级权限（链上） | `4jZq7kXH6XA6xCZvnAvL9NPxN4QFDNHdKfw1zDm1hMwg` == `target/deploy/uxuy_upgrade_authority-keypair.json` ✅ |
| 线上字节 | `e55bbf65…47a1`（617,296 B，slot 460,000,401）— `verify-onchain-elf.sh` **MATCH** |
| ProgramData 容量 | 834,792 B（尾部 217,496 B 零填充） |
| 目标字节 | `da5215e1cb0a0eaf0950f57bd6994945b79595386f9b5a10a871fab5664fa10e`（617,296 B，devnet 已验证） |
| 源码差异 | `b37992b..5a0b08f`：`programs/` 下**只有** meteora 两行（`new_readonly` → `new`），其余为测试/文档/探针 |

### 9.2 预检（均为只读，2026-10-23 17:0x–17:1x 实测）

| 检查 | 结果 |
|---|---|
| 线上 == 回滚件 `uxuy-router-optz-617296.so` | ✅ MATCH（回滚件即当前字节，无需另存） |
| 容量 ≥ 目标 | ✅ 834,792 ≥ 617,296，**不需要 realloc / 不缩容** |
| buffer 租金（`--max-len 617296`） | 4.2973 SOL |
| buffer 租金（不带 `--max-len`） | **5.8110 SOL**（按 834,792 分配）→ 必带 `--max-len 617296` |
| 付费账户 | `uxuy_ops_payer` `HRF4WPP5…S6P` = **4.6390 SOL**（余 0.34 SOL 够手续费） |
| 备用付费账户 | `uxuy_mainnet_payer` `5uuPqko7…L2Z` = **0 SOL**，不可用 |
| 声明 program id | `AN6z…`（devnet 同地址部署后探针全过，反证 id 匹配） |

### 9.3 命令（照抄）

```bash
solana program deploy target/deploy/uxuy-router-devnet-20261023.so \
  --program-id AN6zjJfTD5Cpj2x56sWsAod1Jb5viiQH4Xcg3tm5DgNY \
  --upgrade-authority target/deploy/uxuy_upgrade_authority-keypair.json \
  --keypair target/deploy/uxuy_ops_payer-keypair.json \
  --max-len 617296 \
  --url https://solana-mainnet.g.alchemy.com/v2/<key>
```

> RPC 用 dex 项目 `.env` 里的 Alchemy key（`solana-mainnet` 域名）；公共端点会被限流打断。

### 9.4 预演结果（在 devnet 上用同一条命令形状跑）

- 第一次：`Error: 12 write transactions failed`，程序字节未变（安全失败），残留 buffer
  `3KRi7wKF…`（3.137 SOL）→ `solana program close 3KRi7wKF…` 回收。
- 重跑：**47.8 s 成功**，tx `oSdwTVFJ3Q1gr4eRA8wMWi5bMMvT3ATZjf4afzWBHPcmC4uZQCe2xPxwesK2iF23eAPsrpgaereYx2xkK7hMJrp`；
  字节校验仍 MATCH，ProgramData 容量保持 848,120（不缩容），手续费 ~0.003 SOL。
- 结论：`--max-len` 小于现有容量是安全的（写 buffer，不动 ProgramData 容量）；
  写交易可能瞬时失败 → **主网失败后先 `solana program close <buffer>` 回收再重跑**，
  不要连着重跑（会叠 buffer 占用）。

### 9.5 回滚

```bash
solana program deploy target/deploy/rollback/uxuy-router-optz-617296.so \
  --program-id AN6zjJfTD5Cpj2x56sWsAod1Jb5viiQH4Xcg3tm5DgNY \
  --upgrade-authority target/deploy/uxuy_upgrade_authority-keypair.json \
  --keypair target/deploy/uxuy_ops_payer-keypair.json \
  --max-len 617296 --url https://solana-mainnet.g.alchemy.com/v2/<key>
```

### 9.6 部署后

- `bash scripts/verify-onchain-elf.sh target/deploy/uxuy-router-devnet-20261023.so <mainnet-rpc>` → 期望 MATCH（`da5215e1…`）。
- devnet 探针**不能**用于主网（脚本里的账户/密钥都是 devnet 状态）；主网要验的是
  Gdex 侧 DLMM 段：gdex `3a0b7a3` 起 `bitmapExt` 已传 writable，
  `dex` `origin/uxuy-router` HEAD `dd614cf` 含该改动 —— **先合约后 gdex**，同窗口。

### 9.7 执行结果（2026-10-23 17:22 CST，已上主网）

- 升级 tx `4JwGXLFX4dxMpnBLWTFW8ZhFJGDkWYec1HJwFeUjnWTMLC1ZWy3gjXFcfFE1c5YUpAeBjpPuwRw1bYs8bZMkuSYj`，
  slot **460,387,595**，耗时 1 分 11 秒。
- 链上字节 == `da5215e1cb0a0eaf0950f57bd6994945b79595386f9b5a10a871fab5664fa10e`（`verify-onchain-elf.sh` **MATCH**），
  ProgramData 容量保持 834,792（不缩容），尾部 217,496 B 零填充。
- 付费账户 `uxuy_ops_payer` 4.6390 → **4.6323 SOL**（实际花费 ~0.0068 SOL，buffer 已自动退回）。
- 活性检查：空指令 `simulateTransaction` → `InstructionFallbackNotFound`(101)，新 ELF 正常执行。
- 回滚件：`target/deploy/rollback/uxuy-router-optz-617296.so`（`e55bbf65…`，升级前字节）。
- **gdex 尚未更新**（服务器仍是旧包）：当前处于"合约要可写、gdex 传只读"的中间态，
  下一步等 gdex 部署重启（`dex` `origin/uxuy-router` HEAD `dd614cf`，含 `3a0b7a3` 的 `bitmapExt` 可写）。

### 9.8 升级后闭环验证（2026-10-23 17:5x，主网只读模拟）

用 `scripts/devnet-probes/sim-dlmm-mainnet.js` 打**出过事的那只池子** Kimchi/SOL
`ENpXvFPbJkShkk7UqqXqpEYA9LHpj9Yt4wEofnGNdA9R`（tokenX `CSocnzc…pump` = Token-2022，
tokenY = WSOL，位图扩展 `EhsDKsEv…9WZ` 真实存在）：

```
err: null   CU: 120765
Program log: Dex::MeteoraDlmmSwap2 amount_in: 20000000, offset: 0
Program log: Instruction: Swap2            (meteora LBUZKhRx…)
Program log: SwapEvent { dex: MeteoraDlmmSwap2, amount_in: 20000000, amount_out: 202603661 }
```

0.02 SOL → ~202 token，全程不发送、不花钱。升级前同一条 wire 在这只池子上死在
`ConstraintMut(2000)`（见 `5a0b08f` 提交信息），现在过约束、CPI 完成、meteora 出
`SwapEvent` —— 位图扩展可写的闭环成立。

> 注意：这是 `simulateTransaction`，不是成交；真实成交要等第一笔 DLMM 路由流量。
> 同时 gdex 侧 `dd614cf` 已上线（17:19:39 重启，日志 `dlmm/watch.go` 正常刷 bin）。
