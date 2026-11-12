# magpie + CodeArts ACP 公网代理 — 部署文档

> 生成时间：2026-11-10  
> 容器：cocoonbox-agent (Ubuntu 24.04, XFCE)

## 一、架构总览

```
客户端
  │
  ├── API 调用
  │     POST https://znj6kia5-3425.devbridge-s2.hwtunnel.com/v1/chat/completions
  │         │
  │         ▼
  │     DevBridge 隧道 (znj6kia5, port 3425)
  │         │
  │         ▼
  │     magpie 网关 (:3425)  ── OpenAI 兼容 API
  │         │
  │         ├── model: codearts/*     → AK/SK 直连华为云 MaaS API (积分已耗尽)
  │         │
  │         └── model: codearts-acp/* → ACP bridge (:8099)
  │                                       │
  │                                       ▼
  │                                   CodeArts 工作空间 (WebSocket)
  │                                   wss://{workspace-id}.workspace.developer.huaweicloud.com/codearts-acp/ws
  │
  └── Web UI
        https://znj6kia5-3430.devbridge-s2.hwtunnel.com/?k=<WEB_UI_KEY>
            │
            ▼
        DevBridge 隧道 (znj6kia5, port 3430)
            │
            ▼
        magpie Web UI (:3430)
```

## 二、公网地址

| 用途 | URL |
|------|-----|
| API Base URL | `https://znj6kia5-3425.devbridge-s2.hwtunnel.com/v1` |
| Web UI | `https://znj6kia5-3430.devbridge-s2.hwtunnel.com/?k=<WEB_UI_KEY>` |
| 模型 ID | `codearts-acp/deepseek-v4.1-flash-1017` 等 |

## 三、容器内服务

| 服务 | 端口 | 启动命令 |
|------|------|----------|
| magpie API | 3425 | `./magpie serve --web=0.0.0.0:3430` |
| magpie Web UI | 3430 | 同上（`--web` 参数） |
| ACP bridge | 8099 | `python3 bridge_auto.py`（自动获取 host） |
| DevBridge host | - | `/home/user/.huawei/bin/devbridge host znj6kia5` |

## 四、关键文件

| 文件 | 说明 |
|------|------|
| `/home/user/repos/magpie/magpie` | magpie 二进制 |
| `/home/user/.config/magpie/providers.json` | magpie provider 配置（含 AK/SK） |
| `/home/user/codearts-acp/codearts_acp_bridge.py` | ACP bridge 主程序 |
| `/home/user/codearts-acp/bridge_auto.py` | 自动获取 host 的 bridge 启动脚本 |
| `/home/user/codearts-acp/cookie_new.txt` | 工作空间 cookie（含 hlp_token） |
| `/home/user/db-keepalive.sh` | 保活脚本（magpie + bridge + devbridge） |

## 五、自动获取工作空间 ID

### 原理

cookie 里的 `hlp_token` 是一个 JWT（EdDSA 签名），payload 包含：

```json
{
  "sub": "153bd4023c87405e935df7c0795e7be2",
  "iat": 1794296958,
  "exp": 1794901758,
  "env": "7d47644dc3234cd0a20768c20985c3e4",
  "prt": 5066
}
```

- `env` = 工作空间 UUID
- `prt` = 项目 ID
- 工作空间 hostname = `{prt}-{env}.workspace.developer.huaweicloud.com`
- token 有效期 7 天

### bridge_auto.py 流程

1. 读 `cookie_new.txt`
2. 提取 `hlp_token` JWT
3. 解码 payload，取 `prt` + `env`
4. 拼成 hostname
5. 启动 `codearts_acp_bridge.py --host {hostname}`

### hlp_token 特性

- 绑定到工作空间 UUID，**不**绑定到容器
- 工作空间重建（container rebuild）→ 同一个 token 仍然有效
- 新建工作空间（UUID 变了）→ 需要新 token

## 六、保活机制

`/home/user/db-keepalive.sh` 每 10 秒检查：

1. **magpie**（端口 3425）不在线 → 重启 magpie
2. **ACP bridge**（端口 8099）不在线 → 用 `bridge_auto.py` 重启
3. **DevBridge host** 不在线 → 重启 devbridge host

日志：`/tmp/db-keepalive.log`

## 七、Cookie 更新

### 当前状态

- 自动获取工作空间 ID：✅ 已实现（`bridge_auto.py`）
- 自动刷新 cookie：❌ 未实现

### 为什么不能自动刷新

`hlp_token` 通过浏览器 SSO 登录获取。用 AK/SK 自动获取需要：
- AK/SK → IAM token（STS API 签名复杂，未调通）
- 或 `hcloud` CLI（容器内安装超时）
- 或华为云账号密码（可用于 SSO 模拟）

### 手动更新流程

1. 浏览器打开华为云 CodeArts 工作空间
2. F12 → Application → Cookies → 复制全部 cookie
3. 粘贴到 `/home/user/codearts-acp/cookie_new.txt`
4. 保活脚本会自动用新 cookie 重启 bridge

## 八、magpie Provider 配置

### codearts（直连华为云 MaaS）

- AK/SK：保存在 `/home/user/.config/magpie/providers.json`；文档不重复存放密钥。需要轮换时在华为云控制台操作，并同步更新配置。
- API: `https://snap-access.cn-north-4.myhuaweicloud.com/api/v2`
- 状态：积分已耗尽

### codearts-acp（通过 ACP bridge）

- API: `http://127.0.0.1:8099/v1`
- 流程：magpie → bridge → 工作空间 WebSocket → CodeArts Agent
- 状态：可用（但积分已耗尽）

## 九、可用模型

| 模型 ID | 说明 |
|---------|------|
| `codearts-acp/deepseek-v4.1-flash-1017` | DeepSeek V4.1 Flash |
| `codearts-acp/glm-5.3-flash` | GLM-5.3-Flash |
| `codearts-acp/glm-5.3` | GLM-5.3 |
| `codearts-acp/kimi-k3.1-pro` | Kimi K3.1 Pro |
| `codearts-acp/kimi-k3` | Kimi K3 |
| `codearts-acp/GLM-5.2` | GLM-5.2 |

## 十、故障排查

### Web UI 打不开（service unavailable）

```bash
# 检查进程
ps aux | grep -E "magpie|devbridge|bridge_auto" | grep -v grep

# 检查端口
ss -tlnp | grep -E "3425|3430|8099"

# 重启保活
pkill -f db-keepalive; setsid /home/user/db-keepalive.sh > /dev/null 2>&1 < /dev/null &
```

### 模型调用 404

- bridge 没运行 → 保活脚本会自动拉起
- 工作空间停了 → 浏览器打开 CodeArts 启动工作空间
- cookie 过期 → 更新 `cookie_new.txt`

### 模型返回"积分已耗尽"

- 去 https://console.huaweicloud.com/devcloud/ 领取积分或升级套餐

## 十一、2026-11-10 22:40 更新（ACP 复活 + perch 接入）

### 502 根因（修正旧结论）

- 旧结论"hlp_token 过期"**错误**：旧 token（env 7d47644d…）exp 到 11-17，并未过期。
- 真因：**工作空间实例换了 UUID**（7d47644d… → ef860cc8…），旧 host 网关 502。
- 新 cookie 已写入 cookie_new.txt（env ef860cc8edb64ba9afbf38822184dfae，**exp 2026-11-17 14:36 UTC+8**，届时需再抓）。

### 计费分支（关键发现）

| 分支 | 模型 | 状态 |
|------|------|------|
| huaweicloud-maas/* | glm-5.3、kimi-k3、deepseek-v4.1-flash、speed 等 15 个 | ❌ TM.00001050 积分耗尽（与 AK 路径同一账本）|
| **hwdevspace/*** | **glm-5.1**、glm-5.2、openpangu-2.0-flash | ✅ **不占 doer 积分，实测可用** |

- **同名陷阱**：桥按 id 解析、先到先得。`codearts-acp/glm-5.2` 和 `codearts-acp/openpangu-2.0-flash` 在模型表里有两条（maas 根在前）→ 实际命中 maas 根 → 返回积分错误。**只有 `codearts-acp/glm-5.1` 无重名，确定性命中 hwdevspace 分支。**
- `group/never-stuck` 路由组仍落积分分支，暂不可用。
- maas 分支恢复方法：领取/升级积分 https://console.huaweicloud.com/devcloud/ → doer-credits-apply。

### perch 已接入（byolm，官方命令）

```
perch byolm set http://127.0.0.1:3425/v1 codearts-acp/codearts-acp/glm-5.1
# → reachable · tools no · ctx 32768；Turns now run on your machine: not metered
```

- 链路：perch → magpie(:3425) → bridge(:8099) → wss workspace → hwdevspace/glm-5.1，模型调用零云端。
- 验证：`perch run "Reply with exactly: PERCH-ACP-LIVE" --json` → assistantText 精确回显。
- `tools no` 属预期（ACP agent 不做 OpenAI 工具调用），对话轮正常，agentic 工具执行受限。
- **已开的 perch TUI 会话不会自动切换**（byo 选择在会话启动时读取），需重启 TUI 生效。
- 撤销：`perch byolm clear`。

### 排障补充

- 桥复用单个 ACP 会话：换模型后首次请求可能回显旧口令（历史残留），再发一次即正常。
- `perch byolm set` 探测 ACP 后端耗时约 2 分钟，MCP 120s 上限内跑不完，须 setsid 后台 + 轮询日志。

## 2026-11-10 晚：切换到持久版 f503f9ed（重要）

**当前桥连的是持久版实例** `f503f9ed9f384222b49e9552b9149166`（60 天持久工作空间），不再是体验版 ef860cc8。
- cookie_new.txt 现在只含一行 `hlp_token=<JWT>`（prt=5066, env=f503f9ed, **exp=iat+604800 ≈ 11-17 22:31 UTC+8**，到期重签）
- 体验版备份归档副本在 `archive/credentials/cookie_ef86_1141.txt`；根目录旧副本暂时保留以便回滚。需要切回时，将归档副本复制为 `cookie_new.txt` 后 kill 桥进程（keepalive 10s 自动拉起）。旧令牌均按 600 权限保存。

### JWT 续签（免密码，需浏览器登录态）
1. 浏览器打开 https://devstation.developer.huaweicloud.com/aidevspace （重建 devstation-svc 会话，掉了会报 not login）
2. 页面 console 执行：
   fetch('https://devstation-svc.connect.huaweicloud.com/open-web-user/v1/hyperlane/login?devenv_id=f503f9ed9f384222b49e9552b9149166&port=5066', {credentials:'include', headers:{request_csrf:document.cookie.match(/hd-csrf=([^;]+)/)[1], csrf:document.cookie.match(/hd-csrf=([^;]+)/)[1]}}).then(r=>r.json()).then(d=>console.log(d.result.jwt))
3. 写入 cookie_new.txt → kill 桥 → keepalive 自动重启

### 关键教训（ACP 协议）
- **session/prompt 必须是带 id 的 JSON-RPC request**。无 id 的 notification 会被 agent 静默丢弃（零帧、无错误、像挂死）。initialize/session/new 同理都带 id。
- 19300 直连端口是残缺 agent（Method not found: session/prompt），不要用；5066 的 /codearts-acp/ws 才是完整接口。
- gateway key `full` 已放开全部模型（magpie gateway-key models <id> all）。
