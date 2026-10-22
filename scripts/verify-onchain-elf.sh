#!/usr/bin/env bash
# Verify that a local .so is byte-identical to what is deployed on chain.
#
# The on-chain ProgramData account is 45-byte loader header + ELF + zero
# padding (the account keeps the largest --max-len it ever had, so the
# padding must be ignored and only the first len(local .so) bytes compared).
#
# usage: scripts/verify-onchain-elf.sh <local.so> [rpc-url] [programdata-addr]
set -euo pipefail
SO=${1:?usage: verify-onchain-elf.sh <local.so> [rpc-url] [programdata-addr]}
RPC=${2:-https://api.mainnet-beta.solana.com}
PD=${3:-DvCC2wgbT4TBSD2w4jPsFbGTBHPZnKvJk21Yh4FCHZik}

python3 - "$SO" "$RPC" "$PD" <<'PY'
import base64, hashlib, json, sys, urllib.request

so_path, rpc, pd = sys.argv[1], sys.argv[2], sys.argv[3]
local = open(so_path, "rb").read()
body = json.dumps({
    "jsonrpc": "2.0", "id": 1, "method": "getAccountInfo",
    "params": [pd, {"encoding": "base64"}],
}).encode()
req = urllib.request.Request(rpc, data=body, headers={"Content-Type": "application/json"})
info = json.load(urllib.request.urlopen(req, timeout=30))["result"]["value"]
payload = base64.b64decode(info["data"][0])[45:]  # strip loader header

if len(payload) < len(local):
    print(f"MISMATCH: on-chain payload {len(payload)} B < local {len(local)} B")
    sys.exit(1)

onchain = payload[:len(local)]
h_local = hashlib.sha256(local).hexdigest()
h_chain = hashlib.sha256(onchain).hexdigest()
print(f"local   : {len(local)} B  sha256 {h_local}")
print(f"on-chain: {len(payload)} B payload ({len(payload) - len(local)} B zero padding)  first-{len(local)} sha256 {h_chain}")
print("MATCH" if onchain == local else "MISMATCH")
sys.exit(0 if onchain == local else 1)
PY
