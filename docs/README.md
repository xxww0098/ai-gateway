# 文档导航

长期约束说明“为什么这样分”；审查与性能报告只对其记录的代码、配置和测量环境负责。执行方案放在 specs，不能拿计划中的保证描述当前生产能力。

## 架构与工程

- [架构与项目树](architecture.md)：目录落点、责任边界，以及前端的待实施分层。
- [工程契约](../CONTRACT.md)：现有工程决定和文件所有权。
- [项目树对抗审查](reviews/project-structure.md)：本次审查的证据、裁决与未解决风险。
- [支付确认与部署约束](payment-settlement.md)：原子入账、历史重试和迁移前的对账要求。
- [计费加固方案](../specs/billing-hardening/README.md)：资金一致性的独立实施入口。
- [服务面 API](service-api.md)：ozon-pod 集成的内部接口（建号 / 发 key / 余额 / 用量 / 入账）与幂等规则。

## 请求与上游

- [API 请求全链路时序](api-request-sequence.md)
- [OAuth 参考仓库](oauth.md)
- [OAuth 故障](oauth-error.md)
- [入口收敛设计](relay-surface-plan.md)
- [透传保真度审计](relay-passthrough-audit.md)

## 性能证据

- [中继性能基线](relay-perf-baseline.md)
- [中继验收记录](relay-perf-acceptance.md)
- [热路径火焰图](hotpath-flamegraph.md)
- [与 NewAPI / CLIProxyAPI 的对照](perf-vs-newapi-cliproxy.md)

历史报告中的外部路径、旧规则编号和当时的配置不构成现行工程指令；复测前以当前源码与可执行门禁为准。
