# ShellX

> 带本地 MCP 服务与纵深安全护栏的桌面终端客户端。基于 **Tauri 2 + React + TypeScript + xterm.js**，安全策略在 **Rust 核心**中执行。

ShellX 面向两类使用者：

- **人**：在 GUI 中管理连接、分组、打开多标签终端、手动操作服务器。
- **AI Agent**：通过本地 MCP Server 列出已授权服务器、执行命令、读取输出；所有调用都经过连接级授权、环境分级、风险判定、人工审批与审计日志。

设计依据：[Tauri + 安全 MCP 服务器终端应用设计文档](./Tauri%20+%20安全%20MCP%20服务器终端应用设计文档.md)。

---

## 当前状态：脚手架（v0.1.0）

本仓库目前是**可运行的产品骨架**，不是完整实现：

| 模块 | 状态 | 说明 |
| --- | --- | --- |
| GUI 布局、连接管理、分组、搜索 | ✅ 已实现 | 连接元数据持久化到应用数据目录 |
| 多标签终端（xterm.js） | ✅ 已实现 | 每个连接一个终端页，支持切换/关闭 |
| 策略引擎（风险分级 + 环境矩阵 + MCP 授权） | ✅ 已实现 | Rust 侧执行，含单元测试 |
| 审批弹窗（批准 / 修改后执行 / 拒绝） | ✅ 已实现 | 挂起命令并等待用户决策 |
| 输出脱敏（密码/Token/私钥/Bearer） | ✅ 已实现 | 正则规则位于 `redaction.rs` |
| 审计日志（JSONL + GUI 查看） | ✅ 已实现 | 每日文件 + 内存最近 500 条 |
| 会话注册表 / 输出环形缓冲 | ✅ 骨架 | 内存实现，供后续 SSH 通道挂载 |
| 真实 SSH 通道 | 🚧 待接入 | 当前由**模拟执行器**返回演示输出 |
| 本地 MCP stdio 传输 | 🚧 待接入 | 工具目录、JSON Schema、分发器已就绪 |
| 凭据持久化（系统钥匙串） | 🚧 待接入 | 当前仅内存保存，重启即失效 |

> 终端中的命令标注「模拟」表示未连接真实服务器；策略判定、审批与审计是真实生效的，可在浏览器 Mock 模式或 Tauri 窗口中完整演示。

## 功能演示

1. 左侧选择 `prod-web-01` 打开终端。
2. 点击终端右上角 **手动模式** 切换为 **AI Agent 模式**。
3. 执行 `ls -la`：命中只读白名单，直接放行。
4. 执行 `systemctl restart nginx`：生产环境 + 高风险 → 弹出审批窗口，可批准、修改命令或拒绝。
5. 执行 `rm -rf /`：极高危规则 → 直接拦截并记录审计。
6. 点击顶部 **审计日志** 查看每次判定、审批结果、耗时与退出码。

浏览器直接 `npm run dev` 时使用内置 Mock 后端（界面会显示「浏览器 Mock 模式」）；在 Tauri 中运行时，所有判定与持久化由 Rust 核心完成。

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
  ├─ vault.rs          凭据保险库（当前为内存实现）
  ├─ sessions.rs       会话注册表与输出缓冲
  └─ mcp.rs            MCP 工具目录 / manifest / 分发器骨架
                                       │
                                       v
                              SSH / SFTP（phase 1）
```

### 目录结构

```
shellx/
├─ src/                      React 前端
│  ├─ components/            Sidebar / TerminalPane / 审批 / 审计 / 连接弹窗
│  ├─ lib/api.ts             Tauri 调用封装 + 浏览器 Mock 后端
│  ├─ lib/types.ts           与 Rust serde 模型对应的类型
│  └─ styles.css             暗色终端工作台样式
├─ src-tauri/                Rust 核心（Tauri 2）
│  ├─ src/models.rs          领域模型（camelCase 序列化）
│  ├─ src/policy.rs          安全策略引擎（含单元测试）
│  ├─ src/redaction.rs       输出脱敏（含单元测试）
│  ├─ src/audit.rs           审计日志
│  ├─ src/state.rs           核心服务编排
│  ├─ src/mcp.rs             MCP 工具目录与分发器骨架
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
npm run build        # TypeScript 类型检查 + 生产构建
npm run tauri dev    # 启动 Tauri 桌面端（需要 Rust 工具链）
npm run tauri build  # 打包安装包
```

Rust 侧测试（需要 Rust 工具链）：

```bash
cd src-tauri
cargo test
```

### 数据位置

Tauri 应用数据目录（标识符 `com.shellx.app`）下：

- `connections.json`：连接元数据（不含密码/私钥内容）
- `audit/audit-YYYY-MM-DD.jsonl`：结构化审计日志

Windows 通常位于 `%APPDATA%\com.shellx.app`。

## MCP 工具（骨架已定义）

| 工具 | 风险 | 状态 |
| --- | --- | --- |
| `list_servers` | 低 | 过滤为仅返回已授权连接的安全元信息 |
| `exec_command` | 中 | 走 `AppState::run_command`，策略/审批/审计全生效 |
| `read_output` | 低 | 协议已定义，待接入 |
| `send_signal` | 中 | 协议已定义，待接入 |
| `close_session` | 中 | 协议已定义，待接入 |

stdio JSON-RPC 循环将以 `shellx --mcp-stdio` 形式接入；`src-tauri/src/mcp.rs` 已提供 `tools/list` manifest 与 `tools/call` 分发器。

## 实施路线

1. **第一阶段（MVP）**：接入 `russh` 真实 SSH 通道与 PTY；系统钥匙串保存凭据；实现 MCP stdio 传输；完成 `read_output` / `send_signal` / `close_session`。
2. **第二阶段（终端体验）**：私钥/跳板机链路、分屏、快捷命令、SFTP 文件传输、大输出分页、审计检索。
3. **第三阶段（安全与工程化）**：可视化规则配置、环境策略自定义、人机同屏接管提示、错误恢复与规则回归测试。

## 说明

- 当前凭据仅保存在内存中，属于脚手架行为，请勿用于真实生产环境。
- 模拟执行器不会建立任何网络连接；所有「执行成功」均为本地演示输出。
