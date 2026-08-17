# AGW-Oauth

DeepSeek Harness 插件：在 `dsh` 里直接 OAuth 登录 **AI-GateWay**，不用手写模型配置文件。

## 安装

```bash
dsh plugin --profile web add ./plugins/agw-oauth
```

或：

```bash
dsh plugin --profile web add github:xxww0098/ai-gateway
```

## 设置页

web profile 打开 **设置 → AGW Oauth**。填写网关地址后点 **登录**，页面走现有设备码流程并轮询状态，直到 OAuth 凭据写入。没有新的 OAuth 协议。

命令栏里的 `/agw login`、`/agw status`、`/agw logout` 仍然可用。

## 环境变量

`AGW_ORIGIN`：网关 origin，例如 `https://gw.example.com`。若已在 **设置 → AGW Oauth** 保存网关地址，则不必再设。

登录成功后，origin 与 API Key 写在 `$DSH_HOME/agw-oauth.json`。设置页可以只保存 origin（没有 api_key 时仍视为未登录）。

## 登录流程

1. 在 **设置 → AGW Oauth** 填写网关地址（或设置 `AGW_ORIGIN`），启动 `dsh web`。
2. 点击 **登录**，或执行 `/agw login`。拿到验证 URL / 用户码后立刻返回。
3. 打开 URL，用现有面板账号登录并批准。
4. 插件轮询拿到 `agw-` API Key 和网关 origin，再请求 `{origin}/v1/models`。
5. 模型出现在 `ai-gateway` 路由。`resolveModel()` 会带上目录里的思考档位、`text`/`image` 模态、输入/输出 token 上限；目录没有的字段不会编造。

## 可选 HTTP API

web profile 启动后还会挂（同源）：

- `POST /agw-oauth/login/start`：等同 `/agw login`，立刻返回 URL 和用户码
- `GET /agw-oauth/status`：`{ loggedIn, origin, watch }`（仅保存了设置页 origin 时也会返回 origin）
- `POST /agw-oauth/origin`：`{ "origin": "https://..." }`，不依赖 `AGW_ORIGIN` 持久化网关地址
- `POST /agw-oauth/logout`：等同 `/agw logout`（清空 store、卸掉 adapter）
