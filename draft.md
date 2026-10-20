
# ClientMCP Protocol

**Version:** 0.1.0 Draft

> **Expose live client applications as MCP servers.**

ClientMCP 是一个轻量级协议，允许浏览器、桌面和移动应用通过 WebSocket，将自身功能暴露为标准 MCP Server，供 AI Agent 远程调用。

客户端无需开放端口，也不需要自己运行 HTTP Server。

## 1. 核心架构

```text
           Client Application
          ┌────────────────────┐
  Human ──► UI                 │
          │        Shared      │
          │        State       │
          │          ▲         │
          │          │         │
          │    ClientMCP SDK   │
          └──────────┬─────────┘
                     │
                     │ WebSocket
                     ▼
          ┌────────────────────┐
          │  ClientMCP Bridge  │
          │                    │
          │  Tool Registry     │
          │  Request Routing   │
          │  MCP HTTP Server   │
          └──────────┬─────────┘
                     │
                     │ MCP Streamable HTTP
                     ▼
                  AI Agent
```

ClientMCP 包含三个组件：


| 组件           | 职责                                         |
| ---------------- | ---------------------------------------------- |
| **Client SDK** | 注册应用功能、接收调用、执行操作             |
| **Bridge**     | 管理 WebSocket 连接、转发调用、提供 MCP 服务 |
| **AI Agent**   | 通过标准 MCP 发现并调用应用功能              |

## 2. 核心原则

1. **Client-side Execution**：所有操作都在客户端应用中执行。
2. **Shared State**：UI 和 MCP 使用同一套应用状态与业务逻辑。
3. **Client-Initiated Connection**：客户端主动连接 Bridge，无需开放端口。
4. **Standard MCP**：AI Agent 只使用标准 MCP Streamable HTTP。
5. **No UI Automation**：不依赖截图、鼠标模拟或 DOM 操作。
6. **No Persistence Required**：Bridge 不需要持久化应用状态和消息。
7. **Explicit Authorization**：应用控制哪些操作可以暴露和执行。

## 3. 通信协议

ClientMCP 使用两种传输方式：


| 通信双方           | 协议                |
| -------------------- | --------------------- |
| Client ↔ Bridge   | WebSocket           |
| AI Agent ↔ Bridge | MCP Streamable HTTP |

Bridge 负责两种协议之间的转换。

### 3.1 客户端连接

```text
wss://bridge.example.com/connect
```

客户端建立 WebSocket 连接并完成身份认证。

Bridge 为该连接关联一个 Client ID。

### 3.2 注册能力

客户端主动注册 MCP Tools：

```json
{
  "type": "register",
  "tools": [
    {
      "name": "getBalance",
      "description": "Get account balance",
      "inputSchema": {
        "type": "object",
        "properties": {
          "chainId": { "type": "number" }
        },
        "required": ["chainId"]
      }
    }
  ]
}
```

Bridge 将其映射为 MCP `tools/list` 的结果。

客户端可在运行期间更新或撤销已注册的工具。

### 3.3 Agent 调用

Agent 通过 MCP 调用：

```text
tools/call → getBalance
```

Bridge 转换为 WebSocket 消息：

```json
{
  "type": "invoke",
  "id": "req-123",
  "name": "getBalance",
  "arguments": {
    "chainId": 1
  }
}
```

### 3.4 客户端执行

客户端调用自身业务逻辑，必要时更新共享状态。

执行完成后返回：

```json
{
  "type": "result",
  "id": "req-123",
  "result": {
    "balance": "100.50"
  }
}
```

Bridge 将结果转换为标准 MCP `tools/call` 响应。

`id` 用于关联请求与响应，允许多个请求并发执行。

---

## 4. Client 生命周期

```text
DISCONNECTED
     │
     │ WebSocket Connect
     ▼
CONNECTED
     │
     │ Register Tools
     ▼
READY
     │
     ├── Update Tools → READY
     │
     └── Disconnect → OFFLINE
                       │
                       └── Reconnect → READY
```

- 客户端断开后，Bridge 将其标记为离线。
- 重连后，客户端重新注册能力。
- 离线时不执行工具调用，也不缓存待执行操作。
- Bridge 重启后，客户端自动重连。
- 同一客户端身份的重复连接必须有明确的冲突处理规则。

## 5. UI 状态一致性

**UI 和 MCP 必须调用相同的业务操作，并共享应用状态。**

```text
Human ──► UI ───────┐
                    ▼
              Application
              State & Actions
                    ▲
Agent ──► MCP ──────┘
```

例如：

```typescript
const createTransaction = async (params) => {
  return wallet.createTransaction(params);
};

// Human
sendButton.onclick = () => createTransaction(params);

// Agent
clientmcp.expose("createTransaction", createTransaction);
```

Agent 执行操作后，UI 应反映最新的应用状态。

但 ClientMCP 不负责管理 UI 状态。它要求 SDK 调用应用提供的操作，而不是维护另一套业务实现。

## 6. 安全模型

ClientMCP Bridge 是可信的协议转换组件，不提供端到端加密的不可见性保证。

安全规则：

1. **Client Authentication**：客户端连接必须认证。
2. **Agent Authorization**：Agent 必须获得访问指定客户端的权限。
3. **Tool Permissions**：客户端决定哪些操作可以暴露。
4. **User Confirmation**：敏感操作可要求用户在客户端确认。
5. **Request Validation**：所有参数必须校验。
6. **Execution Timeout**：所有调用必须有超时和取消机制。

Bridge 可以读取 MCP 请求和响应，因此不应宣称服务器无法访问业务数据。

## 7. MCP 兼容性

Bridge 对外实现标准 MCP Streamable HTTP，至少支持：

- `tools/list`
- `tools/call`
- 工具列表变更通知
- 标准 MCP 错误与结果格式

WebSocket 内部消息无需完全复制 MCP 协议，只需保留正确映射所必需的信息。

MCP 的 HTTP 会话、通知和流式响应行为应遵循所选 MCP 版本。特别是 2026-07-28 规范已经调整了 Streamable HTTP 的会话模型，不能直接套用旧版本实现。[GitHub](https://github.com/modelcontextprotocol/modelcontextprotocol/blob/main/docs/specification/2026-07-28/changelog.mdx?utm_source=chatgpt.com)

## 8. Client SDK

应用开发者只需要：

```typescript
const client = new ClientMCP({
  bridge: "wss://bridge.example.com/connect",
  token: "CLIENT_TOKEN"
});

client.expose("getBalance", {
  description: "Get account balance",
  inputSchema: balanceSchema,
  handler: async (args) => wallet.getBalance(args)
});

await client.connect();
```

SDK 自动处理工具注册、调用分发、请求关联及断线重连。

## 9. 设计边界

ClientMCP **不实现**：

- UI 自动化
- 鼠标或键盘模拟
- 应用业务逻辑
- 应用状态管理
- 消息持久化
- 离线任务队列
- PeerConnect 协议

第一版专注 MCP Tools。Resources、Prompts 和流式工具执行可以作为后续扩展。

## 10. Design Philosophy

> **One Application. Two Interfaces. Shared State.**

**UI for humans. MCP for agents.**

ClientMCP 让运行中的客户端应用，同时向人类和 AI Agent 提供原生交互能力。

应用负责功能和状态，ClientMCP 负责连接与协议转换。

---

**ClientMCP Protocol — Draft v0.1.0**

*Expose live client-side capabilities as standard MCP tools, without running an HTTP server on the client.*
