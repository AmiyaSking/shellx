# ShellX

![CI](https://github.com/AmiyaSking/shellx/actions/workflows/ci.yml/badge.svg)

> 带本地 MCP 服务与纵深安全护栏的桌面终端客户端。基于 **Tauri 2 + React + TypeScript + xterm.js**，安全策略在 **Rust 核心**中执行。

ShellX 面向两类使用者：

- **人**：在 GUI 中管理连接、分组、打开多标签终端、手动操作服务器。
- **AI Agent**：通过本地 MCP Server 列出已授权服务器、执行命令、读取输出；所有调用都经过连接级授权、环境分级、风险判定、人工审批与审计日志。

设计依据：[Tauri + 安全 MCP 服务器终端应用设计文档](./Tauri%20+%20安全%20MCP%20服务器终端应用设计文档.md)。

---

## 当前状态（v0.1.0）

真实 SSH 交互已经可用，终端基础体验仍在补齐：

| 模块 | 状态 | 说明 |
| --- | --- | --- |
| GUI 布局、连接管理、分组、搜索 | ✅ 已实现 | 连接元数据持久化到应用数据目录 |
| 多标签终端（xterm.js + 真实 PTY） | ✅ 已实现 | 每个连接一个终端页，支持切换/关闭、窗口自适应 |
| 真实 SSH 通道 | ✅ 已实现 | `russh` + PTY，支持密码、私钥、SSH Agent |
| SSH Agent | ✅ 已实现 | Windows OpenSSH Agent / Pageant，Unix `SSH_AUTH_SOCK` |
| 单级跳板机 | ✅ 已实现 | 通过 `direct-tcpip` 建立 ProxyJump 链路 |
| 凭据持久化 | ✅ 已实现 | Windows Credential Manager / macOS Keychain / Linux keyring |
| 主机密钥校验 | ✅ 已实现 | 首次连接必须确认指纹；密钥变更直接拒绝；可移除旧密钥 |
| 连接超时与取消 | ✅ 已实现 | 最长 120 秒（含 2FA 等待），连接中可取消 |
| 策略引擎（风险分级 + 环境矩阵） | ✅ 已实现 | Rust 侧执行，含命令链/替换/启动器解析与单元测试 |
| 审批弹窗、输出脱敏、审计日志 | ✅ 已实现 | 审批可修改命令；PTY 连接/断开/失败也写入审计 |
| 终端搜索 / 链接 / Unicode / GPU 渲染 | ✅ 已实现 | Search / WebLinks / Unicode11 / WebGL addon |
| 同主机多开、标签快捷键、动态标题 | ✅ 已实现 | “+” 或 `Ctrl+Shift+T` 复制会话 |
| 终端响铃提示 | ✅ 已实现 | 后台标签显示提示点 |
| 标签重命名/拖拽、设置页 | ✅ 已实现 | 双击标签重命名，拖动排序 |
| 右键菜单、多行粘贴确认 | ✅ 已实现 | 复制/粘贴/全选/搜索/清屏 |
| 分屏（多窗格） | ✅ 已实现 | 最多 4 个窗格，可拖动分隔条、双击均分 |
| 会话恢复（工作区） | ✅ 已实现 | 设置中开启后，启动时恢复标签/分屏并自动重连 |
| 终端输出日志 | ✅ 已实现 | 写入 `sessions/日期/会话ID.log`，默认关闭 |
| 终端输出批处理与背压 | ✅ 已实现 | 约 8ms / 64KiB 合并事件；1MiB 未确认窗口，UI 落后时反向节流远端 |
| OpenSSH config 导入 | ✅ 已实现 | `~/.ssh/config` 预览选择导入，支持单级 ProxyJump 映射 |
| 端口转发（-L / -R / -D） | ✅ 已实现 | 规则持久化，手动启动/停止，每规则独立 SSH 连接 |
| SFTP 文件浏览 / 上传 / 下载 | ✅ 已实现 | 独立会话、目录操作（目录递归删除）、流式传输、进度显示与取消 |
| 每连接 TERM / 编码 / 启动命令 | ✅ 已实现 | UTF-8 / GBK / GB18030 / Big5，登录后自动执行 |
| keyboard-interactive / 2FA | ✅ 已实现 | 全局弹窗；终端、端口转发、SFTP、跳板机均支持 |
| 旧算法兼容 | ✅ 已实现 | 每连接可选 ssh-rsa / SHA-1 KEX/MAC / CBC / 3DES |
| SSH Agent 转发 | ✅ 已实现 | 每连接可选，OpenSSH Agent / `SSH_AUTH_SOCK` |
| X11 转发 | 🚧 暂未实现 | Windows 场景价值较低 |
| 本地 MCP stdio 传输 | ✅ 已实现 | `shellx --mcp-stdio`；JSON-RPC 2.0，工具调用走策略/审批/审计 |

> 桌面版连接真实服务器；浏览器 `npm run dev` 会使用内置 Mock 后端，界面显示「浏览器 Mock 模式」。

## 功能演示（桌面版）

1. 左侧新建连接，选择密码、私钥或 SSH Agent 认证。
2. 首次连接会弹出主机密钥指纹确认窗口；确认后才写入 ShellX 自己的 `known_hosts`。
3. 连接成功后进入真实交互式 shell；窗口缩放会同步远端 PTY 尺寸。
4. 连接中可点击 **取消**；断线后点击 **重新连接**。
5. 密钥变更时会拒绝连接，并提供「移除该主机密钥并重试」入口。
6. 点击顶部 **审计日志** 查看连接、断开、失败记录。

浏览器 Mock 模式主要用于预览 UI 和策略/审批流程，不建立任何网络连接。

### 终端快捷键

| 快捷键 | 功能 |
| --- | --- |
| `Ctrl+Shift+F` | 搜索终端输出 |
| `Ctrl+Shift+T` | 复制当前会话（同主机新标签） |
| `Ctrl+Shift+D` | 左右分屏 |
| `Ctrl+Shift+E` | 上下分屏 |
| `Ctrl+Shift+W` | 关闭当前窗格（最后一个窗格则关闭标签） |
| `Ctrl+Tab` / `Ctrl+Shift+Tab` | 下一个 / 上一个标签 |
| `Ctrl+1` … `Ctrl+9` | 跳到指定标签 |
| `Ctrl+Shift+=` / `Ctrl+Shift+-` | 放大 / 缩小字号 |
| `Ctrl+Shift+0` | 重置字号 |
| `Ctrl+Shift+C` / `Ctrl+Shift+V` | 复制 / 粘贴 |

URL 使用系统默认浏览器打开（通过 Tauri opener 插件）。终端内右键可打开复制/粘贴/全选/搜索/清屏菜单；多行粘贴会在设置开启时先弹出确认。双击标签可重命名，拖动标签可排序。设置保存在本机 localStorage。

分屏：同一标签内可用工具栏的 ◫ / ⬒ 按钮或 `Ctrl+Shift+D` / `Ctrl+Shift+E` 创建左右/上下窗格，点击窗格切换焦点，`Ctrl+Shift+W` 关闭当前窗格。每个窗格拥有独立 SSH 会话；最多 4 个窗格、同方向平铺。拖动分隔条可调整比例，双击分隔条恢复均分。暂不支持左右+上下嵌套的混合布局。

## 架构

```
Tauri Frontend (React + TS + xterm.js)
  ├─ 连接管理器 / 终端工作台 / 审批弹窗 / 审计面板
  └─ src/lib/api.ts  ── Tauri invoke ──┐
                                       v
Tauri Backend / Local Core (Rust)
  ├─ commands.rs       Tauri Commands
  ├─ state.rs          应用状态与命令编排
  ├─ policy.rs         连接白名单 / 环境分级 / 风险规则
  ├─ redaction.rs      输出敏感信息脱敏
  ├─ audit.rs          JSONL 审计 + 内存环形缓冲
  ├─ connections.rs    连接配置持久化（不含凭据）
  ├─ vault.rs          凭据保险库（系统凭据存储 + 内存缓存）
  ├─ ssh.rs            russh 连接 / 主机密钥 / Agent / 跳板机 / PTY 通道
  ├─ ssh_config.rs     OpenSSH config 只读解析（导入预览）
  ├─ forwards.rs       本地/远程/SOCKS5 端口转发与规则持久化
  ├─ sftp.rs           SFTP 文件浏览、目录操作、流式传输与进度
  ├─ pty.rs            PTY 会话表 / 事件桥 / 输出批处理与背压 / 超时与取消
  ├─ shell.rs          命令链解析（策略引擎输入）
  ├─ encoding.rs       PTY 字节 base64 传输
  ├─ sessions.rs       命令型会话注册表与输出缓冲
  ├─ mcp.rs            MCP 工具目录 / JSON Schema / 分发器
  └─ mcp_stdio.rs      stdio JSON-RPC 2.0 传输（无界面模式）
                                       │
                                       v
                          SSH（russh + PTY + ProxyJump）
```

### 目录结构

```
shellx/
├─ src/                      React 前端
│  ├─ components/            Sidebar / TerminalPane / HostKeyDialog / 审批 / 审计 / 连接弹窗
│  ├─ lib/api.ts             Tauri 调用封装 + 浏览器 Mock 后端
│  ├─ lib/types.ts           与 Rust serde 模型对应的类型
│  └─ styles.css             暗色终端工作台样式
├─ src-tauri/                Rust 核心（Tauri 2）
│  ├─ src/models.rs          领域模型（camelCase 序列化）
│  ├─ src/ssh.rs             SSH/PTY 传输与主机密钥策略
│  ├─ src/pty.rs             会话管理、事件桥、输出批处理与背压、超时/取消
│  ├─ src/shell.rs           命令链解析
│  ├─ src/policy.rs          安全策略引擎（含单元测试）
│  ├─ src/redaction.rs       输出脱敏（含单元测试）
│  ├─ src/audit.rs           审计日志
│  ├─ src/state.rs           核心服务编排
│  ├─ src/mcp.rs             MCP 工具目录与分发器
│  ├─ src/mcp_stdio.rs       stdio JSON-RPC 传输（无界面模式）
│  ├─ capabilities/          Tauri 权限能力
│  └─ tauri.conf.json
├─ assets/app-icon.png       图标源文件
├─ public/app-icon.png       favicon
└─ 设计文档
```

## 安全模型

安全控制**不依赖 Prompt 自律**，全部在 Rust 侧统一执行：

- **默认最小权限**：新连接默认 `mcpAccess = disabled`。
- **连接级授权**：`read_only` 仅允许低风险命令；`standard_controlled` 走环境策略矩阵；手动 GUI 会话不受 MCP 授权限制。
- **环境分级**：

  | 环境 | 低风险 | 中风险 | 高风险 | 极高危 |
  | --- | --- | --- | --- | --- |
  | dev | 放行 | 放行 | 审批 | 拦截 |
  | staging | 放行 | 审批 | 审批 | 拦截 |
  | prod | 放行 | 审批 | 审批 | 拦截 |

- **硬拦截**：`rm -rf /`、磁盘格式化/裸写、fork 炸弹等即使人工批准也不会执行。
- **审批流**：挂起命令、展示目标/环境/完整命令/风险原因，支持修改后执行；修改后的命令会重新过策略。
- **输出脱敏**：返回给 Agent 或写入审计前，对密码、Token、API Key、私钥块、Bearer 凭据做掩码。
- **审计**：记录来源（用户/Agent）、服务器、命令、风险、判定、审批结果、耗时与退出码。

## 开发

### 前置要求

- Node.js ≥ 20.19（推荐 22+）
- Rust stable ≥ 1.77.2（仅运行 Tauri 桌面端时需要）
- 平台依赖：Windows 需 WebView2 与 MSVC Build Tools；macOS 需 Xcode CLT；Linux 需 WebKitGTK 等（见 Tauri 文档）

### 常用命令

```bash
npm install          # 安装前端依赖
npm run dev          # 浏览器预览（自动使用 Mock 后端）
npm test             # 前端单元测试（Vitest + jsdom）
npm run build        # TypeScript 类型检查 + 生产构建
npm run tauri dev    # 启动 Tauri 桌面端（需要 Rust 工具链）
npm run tauri build  # 打包安装包
```

Rust 侧测试（需要 Rust 工具链）：

```bash
cd src-tauri
cargo test
```

### 测试与 CI

- **前端**：Vitest + jsdom，覆盖设置持久化/默认值迁移与工作区序列化；`npm test`
- **Rust**：策略引擎、命令解析、脱敏、SSH config、编码、旧算法、SFTP 传输取消等单元测试；`cargo test`
- **SSH / SFTP 集成测试**：进程内 `russh` 服务端 + 真实 loopback SSH 连接，覆盖主机密钥首次确认、密码认证、keyboard-interactive/2FA、PTY 回显、`direct-tcpip` 转发、`exec` 输出/退出码/超时/运行中信号，以及 SFTP 目录列举、创建/重命名/递归删除与 256KiB 文件读写往返；无需 Docker 或外部 sshd
- **MCP stdio 集成测试**：直接驱动 JSON-RPC 循环，覆盖 `initialize`、`tools/list`、授权过滤、策略拦截、审批自动拒绝与审计、真实 SSH `exec_command`、`wait=false` 后台执行、`read_output` 分页与状态轮询、`send_signal` 与 `close_session`
- **CI**：`.github/workflows/ci.yml` 在 Windows 上执行前端测试/构建 + `cargo test` / `cargo check`
- 系统凭据存储的读写测试标记为 `#[ignore]`，需要手动运行：
  `cargo test keyring_roundtrip -- --ignored`
- 尚未包含前端 E2E；后续可补齐

### 数据位置

Tauri 应用数据目录（标识符 `com.shellx.app`）下：

- `connections.json`：连接元数据（不含密码/私钥内容）
- `known_hosts`：ShellX 自己的主机密钥信任库（不修改 OpenSSH 的 `~/.ssh/known_hosts`）
- `audit/audit-YYYY-MM-DD.jsonl`：结构化审计日志
- `sessions/YYYY-MM-DD/<session-id>.log`：开启「记录终端输出」后的原始 PTY 输出
- `forwards.json`：端口转发规则

密码与私钥口令存放在操作系统凭据存储中，不落盘到 JSON。Windows 通常位于
`%APPDATA%\com.shellx.app`。

## MCP 工具（stdio 已接入）

| 工具 | 风险 | 状态 |
| --- | --- | --- |
| `list_servers` | 低 | 已实现：仅返回已授权连接的安全元信息 |
| `exec_command` | 中 | 已实现：真实 SSH `exec`，受策略/审批/审计约束，支持超时、输出截断与 `wait=false` 后台执行 |
| `read_output` | 低 | 已实现：按行分页读取会话输出，附带 `running` / `exitCode` 状态 |
| `send_signal` | 中 | 已实现：向运行中的命令发送 `INT` / `TERM` / `KILL` 等信号 |
| `close_session` | 中 | 已实现：关闭指定命令会话（运行中的命令会先收到 `KILL`） |

以 stdio 方式启动（MCP 客户端会自行拉起该进程）：

```bash
shellx --mcp-stdio
# 可选：指定与桌面版一致的数据目录
shellx --mcp-stdio --data-dir "$APPDATA/com.shellx.app"
```

客户端配置示例：

```json
{
  "mcpServers": {
    "shellx": {
      "command": "C:/path/to/shellx.exe",
      "args": ["--mcp-stdio"]
    }
  }
}
```

- 传输为换行分隔的 JSON-RPC 2.0，stdout 只输出协议消息；日志写入 stderr。
- 支持 `initialize`、`tools/list`、`tools/call`、`ping` 与 `notifications/initialized`。
- `exec_command` 每次使用独立 SSH 连接，默认超时 30 秒（最大 600 秒），返回退出码、输出与是否截断；输出超过 256 KiB 会截断。
- `exec_command` 传 `wait: false` 时立即返回 `sessionId`，命令在后台继续执行；随后可用 `read_output` 轮询 `running` / `exitCode`，用 `send_signal` 发送信号，用 `close_session` 终止。默认 `wait: true` 会等命令结束再返回完整结果。
- 同一连接同一时间只运行一条命令；需要并行执行时请先在桌面端确认主机密钥并建立多个连接。
- 主机密钥必须先在桌面端终端连接中确认；stdio 进程与桌面版共用 `known_hosts`、`connections.json` 与审计目录。
- stdio 模式没有人工审批窗口：需要审批的命令会被**自动拒绝**并写入审计日志，请先在桌面端批准执行或改用低风险命令。

## 实施路线

1. **第一阶段（连接可靠性）**：✅ 真实 SSH/PTY、SSH Agent、单级跳板机、系统钥匙串、主机密钥确认、连接超时/取消、PTY 会话审计。
2. **第二阶段（终端体验）**：✅ 搜索、可点击链接、Unicode 宽度、GPU 渲染、快捷键、同主机多开、动态标题、响铃提示、设置页、右键菜单、多行粘贴确认、标签重命名/拖拽、多窗格分屏、可拖动分隔条、工作区恢复、终端输出日志；🚧 嵌套分屏。
3. **第三阶段（专业能力）**：✅ OpenSSH config 导入、端口转发（-L / -R / -D）、SFTP 文件浏览/上传/下载、每连接 TERM/编码/启动命令、keyboard-interactive/2FA、旧算法兼容、SSH Agent 转发；🚧 X11 转发。
4. **第四阶段（工程化）**：✅ Windows CI（前端测试/构建 + Rust 测试）；✅ 进程内 SSH / SFTP 集成测试；✅ PTY 输出批处理与背压；✅ MCP stdio 传输与工具调用；🚧 前端 E2E、规则配置界面。

## 说明

- 凭据保存在系统凭据存储；Linux 当前使用内核 keyring，重启后可能需要重新输入，后续可切换到 Secret Service。`shellx --mcp-stdio` 以独立进程读取同一个凭据存储，Linux 内核 keyring 在跨进程/重启场景下可能读不到口令。
- `shellx --mcp-stdio` 使用与桌面版相同的应用数据目录（可用 `--data-dir` 或 `SHELLX_DATA_DIR` 覆盖）；主机密钥必须先在桌面端终端连接中确认。stdio 模式没有审批弹窗，需要人工审批的命令会被自动拒绝并记入审计。
- 浏览器 Mock 模式不会建立任何网络连接；桌面版才会连接真实服务器。
- 当前暂不支持多级跳板机、嵌套混合分屏、X11 转发。
- SSH Agent 转发只对交互式终端会话请求；端口转发/SFTP 不会请求 Agent 通道。Windows 转发依赖 OpenSSH Authentication Agent 服务，Pageant 仅用于认证、不支持转发。
- 每连接可设置 TERM 类型与字符编码（UTF-8 / GBK / GB18030 / Big5），修改后在下一次连接生效；输出日志记录远端原始编码字节。
- 「登录后启动命令」会在 shell 建立后自动输入并回车，请只配置可信命令。
- keyboard-interactive/2FA 使用全局认证弹窗，终端、端口转发、SFTP 和跳板机连接都可复用；多个并发认证请求会排队显示，连接取消或超时会自动关闭对应弹窗。
- 「兼容旧算法」默认关闭，开启后会加入 ssh-rsa、SHA-1 KEX/MAC、CBC 与 3DES；仅用于老旧服务器/网络设备，会降低连接安全性。
- SFTP 会话独立于终端会话；主机密钥必须先在终端连接中确认。上传会覆盖同名远端文件；删除目录会递归删除其中的全部内容（会先弹出确认）；上传/下载可随时取消，取消后可能留下部分本地或远端文件；重命名采用 SFTP RENAME，目标存在时的行为取决于服务器。
- 端口转发规则在「转发」面板管理；主机密钥必须先在终端连接中确认。每条运行中的规则使用独立 SSH 连接，不共享终端会话。
- `-R` 的非回环监听需要服务器端允许 `GatewayPorts`；`-D` 当前为无认证 SOCKS5，建议只监听 `127.0.0.1`。
- SSH config 导入为一次性预览：`Include` 不会展开，`Match` 与通配符 Host 会跳过；`ProxyJump` 仅在目标同时被导入或已存在时映射，多级跳转按直连导入并提示。
- 「启动时恢复工作区」默认关闭；开启后会在应用启动时使用已保存连接自动重连。
- 「记录终端输出」默认关闭；日志可能包含敏感输出，请自行控制访问权限。
- PTY 输出会按约 8ms / 64KiB 合并后发送到前端，减少高频小包造成的 IPC 与重绘开销；每个会话有 1MiB 未确认窗口，xterm 解析完一批后回执。UI 处理不过来时后端会暂停读取，russh 内部队列填满后由 TCP 反向节流远端进程。
