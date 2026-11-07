# Usage

This repository is the standalone open-source source of truth for BGXiong third-party adapters:
https://github.com/IM-DaXiong/bgxiong-third-party-adapters

It was previously published as `bgxiong-vendor-adapters`; that name now points here. This repo does not contain the main application; the app imports plugins you deliver under the host contract. See [LICENSE](../LICENSE) (not MIT).

1. Install [BGXiong](https://www.bgxiong.com)
2. Follow [DEVELOPING.md](DEVELOPING.md) (or import a built pack). Start at [SDK V1](../contract/README.md). [SDK V2](../contract/v2/README.md) is a developer preview (`sdkReady` is false)
3. Import or install the package in Settings → Third-party adapters
4. Configure vendor API credentials and run a real probe/trial. Install success is not vendor-side render proof.

For agents: provide this repository plus the vendor's official API docs. Enter secrets in the client at import time, not in source. Start from root [AGENT.md](../AGENT.md) and [LICENSE](../LICENSE). Package index: [packages/README.zh-CN.md](../packages/README.zh-CN.md).
