# 热修部署手册：proxy 入金不变量（`fafecf4`）

## 0. 结论先行

- 线上程序已用**字节比对**确认 = `71ee5a9`（2026-10-22 13:44 本机构建、13:45 部署）。
- 本热修 = `71ee5a9` + 单提交 `fafecf4`（P0-1：`SourceTokenSaIsSource` /
  `SourceTokenSaNotCredited`），**不携带任何 phase-2 内容**。
- 因此**可以单独升级，不需要 gdex 同窗口**（`97f5be0` 的 pump PDA-user 统一已在线上）。

## 1. 产物

| 用途 | 文件 | 字节 | sha256 |
|---|---|---|---|
| 线上基线（回滚用） | `target/deploy/rollback/71ee5a9-live-652304.so` | 652,304 | `685fb20bf69406f9d63e5dcfcb353441849f75b338677114402b1c86eb365ec4` |
| 热修部署件（strip） | `target/deploy/rollback/fafecf4-hotfix-654824.so` | 654,824 | `61be408bc66a5227a723825b397f2effb4f3e61f5c109efffb207210bd2fa26a` |
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
   rent-exempt（655 KB ≈ **4.56 SOL**，升级完成后退回）。
   - `uxuy_ops_payer`（`HRF4WPP5…`）当前 **3.647 SOL → 不足**，需补 ≥ 1 SOL；
   - `uxuy_mainnet_payer`（`5uuPqko7…`）当前 **0 SOL**。
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

**devnet 特殊说明**：devnet 当前链上 ELF（848,120 B，sha256 `220912f8…`）在本机没有
字节一致的副本（`/tmp/devnet_deployed.so` 同尺寸但哈希不同，属跨机构建差异）。
升级 devnet 后如需回滚，只能部署 `71ee5a9` 基线（功能等价的 pre-phase-2 版本），
不能回到"今天 devnet 上那一份"。对测试网可接受，但要在升级前知会 QA。

## 6. 不在本热修范围

phase-2 全部内容（AMM 27 槽、v2 29/28 窗口、`create_pump_hop0_temp`、hop0 入资校验、
ATA 三 seed、v2 token 差额转发、claim 水位、ToB 退款、空路由/trim 防护）都不在这里。
它们带 3 处 wire 变更，必须与 gdex 同窗口另行发布（见 `UXUY_ROUTER_INTEGRATION.md` §7）。

## 7. 本手册的事实依据

- 线上 ProgramData `DvCC2wgb…`：834,837 B = 45 B 头 + 834,792 B 负载；
  负载前 652,304 B 与 `fafecf4^`（= `71ee5a9`）本机构建产物**逐字节相同**；
  其余 182,488 B 为零填充（历史 `--max-len` 余量，不是"未 strip"）。
- 构建可复现：`71ee5a9` 与 `fafecf4` 两棵树在本机重建均得到上表哈希。
