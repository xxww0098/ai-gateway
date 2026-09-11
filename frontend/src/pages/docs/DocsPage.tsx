import { Link, Navigate, useParams } from 'react-router-dom'
import { ArrowRight, KeyRound, LogIn, Send, Server } from 'lucide-react'
import { useAuthStore } from '@/features/auth/auth_store'
import { isDocsSlug } from '@/shared/routes/docs'
import { userRoutes } from '@/shared/routes/user'
import { DocsLayout } from './DocsLayout'
import { CopyField } from './CopyField'
import {
  KEY_PREFIX,
  PRODUCT_NAME,
  anthropicBaseUrl,
  anthropicEnv,
  firstRequestCurl,
  gatewayOrigin,
  openaiBaseUrl,
  openaiEnv,
  pythonOpenaiSnippet,
  sampleKey,
} from './guide'

export default function DocsPage() {
  const { slug } = useParams<{ slug?: string }>()
  if (slug !== undefined && !isDocsSlug(slug)) {
    return <Navigate to="/docs" replace />
  }

  return (
    <DocsLayout>
      {slug === undefined ? <Overview /> : null}
      {slug === 'quickstart' ? <Quickstart /> : null}
      {slug === 'openai' ? <OpenAiClient /> : null}
      {slug === 'claude' ? <ClaudeClient /> : null}
      {slug === 'codex' ? <CodexClient /> : null}
      {slug === 'cursor' ? <CursorClient /> : null}
    </DocsLayout>
  )
}

function KeysCta() {
  const token = useAuthStore((s) => s.token)
  const to = token ? userRoutes.keys : '/login'
  const label = token ? '去创建密钥' : '登录后创建密钥'
  return (
    <Link to={to} className="btn btn-primary rounded-full px-5">
      {label}
      <ArrowRight className="h-4 w-4" />
    </Link>
  )
}

function OriginNote({ origin }: { origin: string }) {
  return (
    <p className="text-sm leading-relaxed text-muted-foreground">
      下方地址默认取当前站点 <code className="rounded bg-muted px-1.5 py-0.5 font-mono text-xs text-primary">{origin || 'window.location.origin'}</code>
      。生产环境请换成你实际部署 {PRODUCT_NAME} 的主机。
    </p>
  )
}

function Overview() {
  const origin = gatewayOrigin()
  const openai = openaiBaseUrl(origin)
  const anthropic = anthropicBaseUrl(origin)

  return (
    <article className="max-w-4xl space-y-10">
      <section className="space-y-4">
        <h2 className="text-lg font-bold tracking-tight text-foreground">网关地址</h2>
        <OriginNote origin={origin} />
        <div className="grid gap-4 md:grid-cols-2">
          <div className="rounded-2xl border border-border/80 bg-card p-6 shadow-2xs">
            <div className="mb-3 flex items-center gap-2 text-sm font-bold text-foreground">
              <Server className="h-4 w-4 text-primary" />
              OpenAI 兼容
            </div>
            <CopyField label="Base URL" value={openai} />
            <p className="mt-3 text-xs leading-relaxed text-muted-foreground">
              对话走 <code className="rounded bg-muted px-1 py-0.5 font-mono text-foreground">POST /v1/chat/completions</code>
              ，也提供 <code className="rounded bg-muted px-1 py-0.5 font-mono text-foreground">POST /v1/responses</code> 与{' '}
              <code className="rounded bg-muted px-1 py-0.5 font-mono text-foreground">GET /v1/models</code>。鉴权：
              <code className="rounded bg-muted px-1 py-0.5 font-mono text-foreground"> Authorization: Bearer {KEY_PREFIX}...</code>
            </p>
          </div>
          <div className="rounded-2xl border border-border/80 bg-card p-6 shadow-2xs">
            <div className="mb-3 flex items-center gap-2 text-sm font-bold text-foreground">
              <Server className="h-4 w-4 text-primary" />
              Anthropic / Claude
            </div>
            <CopyField label="Base URL" value={anthropic} />
            <p className="mt-3 text-xs leading-relaxed text-muted-foreground">
              与 OpenAI 同一主机。Claude / Anthropic SDK 会自行拼接{' '}
              <code className="rounded bg-muted px-1 py-0.5 font-mono text-foreground">/v1/messages</code>，因此 Base URL 填裸{' '}
              <code className="rounded bg-muted px-1 py-0.5 font-mono text-foreground">{'{origin}'}</code>，不要再加{' '}
              <code className="rounded bg-muted px-1 py-0.5 font-mono text-foreground">/v1</code>。租户鉴权同样是 Bearer{' '}
              <code className="rounded bg-muted px-1 py-0.5 font-mono text-foreground">{KEY_PREFIX}...</code>
              （请用 <code className="rounded bg-muted px-1 py-0.5 font-mono text-foreground">ANTHROPIC_AUTH_TOKEN</code>）。
            </p>
          </div>
        </div>
      </section>

      <section className="space-y-4">
        <h2 className="text-lg font-bold tracking-tight text-foreground">三步接入</h2>
        <ol className="grid gap-4 md:grid-cols-3">
          <li className="rounded-2xl border border-border/80 bg-card p-6 shadow-2xs">
            <div className="mb-3 flex h-9 w-9 items-center justify-center rounded-xl bg-primary/10 text-primary">
              <LogIn className="h-4 w-4" />
            </div>
            <div className="font-mono text-xs font-bold uppercase tracking-wider text-muted-foreground">01</div>
            <h3 className="mt-1 font-bold text-foreground">注册 / 登录</h3>
            <p className="mt-2 text-sm leading-relaxed text-muted-foreground">
              在本站创建账号并登录控制台。
            </p>
          </li>
          <li className="rounded-2xl border border-border/80 bg-card p-6 shadow-2xs">
            <div className="mb-3 flex h-9 w-9 items-center justify-center rounded-xl bg-primary/10 text-primary">
              <KeyRound className="h-4 w-4" />
            </div>
            <div className="font-mono text-xs font-bold uppercase tracking-wider text-muted-foreground">02</div>
            <h3 className="mt-1 font-bold text-foreground">创建密钥</h3>
            <p className="mt-2 text-sm leading-relaxed text-muted-foreground">
              打开{' '}
              <Link to={userRoutes.keys} className="font-mono text-xs font-semibold text-primary hover:underline">
                /keys
              </Link>
              ，生成一把 {KEY_PREFIX} 前缀的 API Key。
            </p>
          </li>
          <li className="rounded-2xl border border-border/80 bg-card p-6 shadow-2xs">
            <div className="mb-3 flex h-9 w-9 items-center justify-center rounded-xl bg-primary/10 text-primary">
              <Send className="h-4 w-4" />
            </div>
            <div className="font-mono text-xs font-bold uppercase tracking-wider text-muted-foreground">03</div>
            <h3 className="mt-1 font-bold text-foreground">发第一请求</h3>
            <p className="mt-2 text-sm leading-relaxed text-muted-foreground">
              请求时以 Bearer 方式携带密钥访问 <code className="rounded bg-muted px-1 py-0.5 font-mono text-xs text-foreground">/v1/chat/completions</code>。余额不足时网关返回 402。
            </p>
          </li>
        </ol>
        <CopyField label="curl 示例" value={firstRequestCurl(origin)} multiline />
        <div className="pt-2">
          <KeysCta />
        </div>
      </section>

      <section className="space-y-4">
        <h2 className="text-lg font-semibold tracking-tight text-foreground">查询额度</h2>
        <p className="text-sm leading-relaxed text-muted-foreground">
          同一把 <code className="rounded bg-muted px-1 py-0.5 font-mono text-xs text-foreground">{KEY_PREFIX}...</code> 密钥可{' '}
          <code className="rounded bg-muted px-1 py-0.5 font-mono text-xs text-foreground">GET /v1/usage</code>
          查询钱包余额和今日按模型的 token 消耗。鉴权与{' '}
          <code className="rounded bg-muted px-1 py-0.5 font-mono text-xs text-foreground">GET /v1/models</code> 相同，GET 不计费，响应是裸 JSON，不是面板信封。
        </p>
        <CopyField
          label="curl"
          value={[
            `curl -sS ${origin}/v1/usage \\`,
            `  -H "Authorization: Bearer ${sampleKey()}"`,
          ].join('\n')}
          multiline
        />
        <p className="text-sm leading-relaxed text-muted-foreground">
          OpenAI 兼容客户端的 Base URL 已含 <code className="rounded bg-muted px-1 py-0.5 font-mono text-xs text-foreground">/v1</code>
          ，请请求 <code className="rounded bg-muted px-1 py-0.5 font-mono text-xs text-foreground">{openai}/usage</code>
          。Claude Code 的 Base URL 是裸主机，请请求{' '}
          <code className="rounded bg-muted px-1 py-0.5 font-mono text-xs text-foreground">{anthropic}/v1/usage</code>。
        </p>
        <p className="text-sm leading-relaxed text-muted-foreground">
          返回裸 JSON：<code className="rounded bg-muted px-1 py-0.5 font-mono text-xs text-foreground">balance_usd</code> 是已扣在途预扣后的钱包余额；
          <code className="rounded bg-muted px-1 py-0.5 font-mono text-xs text-foreground">models</code> 是当地今日零点以来按模型折叠的 token 统计（
          <code className="rounded bg-muted px-1 py-0.5 font-mono text-xs text-foreground">tokens_in</code> / <code className="rounded bg-muted px-1 py-0.5 font-mono text-xs text-foreground">tokens_out</code> /{' '}
          <code className="rounded bg-muted px-1 py-0.5 font-mono text-xs text-foreground">tokens</code> / <code className="rounded bg-muted px-1 py-0.5 font-mono text-xs text-foreground">requests</code>
          ）。没有用量时 <code className="rounded bg-muted px-1 py-0.5 font-mono text-xs text-foreground">models</code> 为空数组。本接口不返回订阅配额或邮箱。
        </p>
      </section>
    </article>
  )
}

function Quickstart() {
  const origin = gatewayOrigin()
  return (
    <article className="max-w-4xl space-y-6">
      <OriginNote origin={origin} />
      <div className="space-y-6">
        <div className="rounded-2xl border border-border/80 bg-card space-y-4 p-6 shadow-2xs">
          <h2 className="text-base font-bold tracking-tight text-foreground">OpenAI 兼容</h2>
          <CopyField label="环境变量" value={openaiEnv(origin)} multiline />
          <p className="text-sm leading-relaxed text-muted-foreground">
            SDK 与 Cursor、Codex CLI 等都把 Base URL 设为{' '}
            <code className="rounded bg-muted px-1.5 py-0.5 font-mono text-xs text-foreground">{openaiBaseUrl(origin) || '{origin}/v1'}</code>
            ，密钥填 <code className="rounded bg-muted px-1.5 py-0.5 font-mono text-xs text-foreground">{KEY_PREFIX}...</code>。
          </p>
        </div>
        <div className="rounded-2xl border border-border/80 bg-card space-y-4 p-6 shadow-2xs">
          <h2 className="text-base font-bold tracking-tight text-foreground">Anthropic / Claude Code</h2>
          <CopyField label="环境变量" value={anthropicEnv(origin)} multiline />
          <p className="text-sm leading-relaxed text-muted-foreground">
            Base URL 是裸主机（SDK 会请求 <code className="rounded bg-muted px-1.5 py-0.5 font-mono text-xs text-foreground">/v1/messages</code>
            ）。本网关只认 <code className="rounded bg-muted px-1.5 py-0.5 font-mono text-xs text-foreground">Authorization: Bearer</code>，请设置{' '}
            <code className="rounded bg-muted px-1.5 py-0.5 font-mono text-xs text-foreground">ANTHROPIC_AUTH_TOKEN</code>。
          </p>
        </div>
        <CopyField label="第一请求" value={firstRequestCurl(origin)} multiline />
        <p className="text-sm leading-relaxed text-muted-foreground">
          计费在请求进入上游前预扣；余额不足时返回 HTTP 402，不会发出上游调用。
        </p>
        <KeysCta />
      </div>
    </article>
  )
}

function OpenAiClient() {
  const origin = gatewayOrigin()
  return (
    <article className="max-w-4xl space-y-6">
      <div className="space-y-6">
        <CopyField label="环境变量" value={openaiEnv(origin)} multiline />
        <CopyField label="Python" value={pythonOpenaiSnippet(origin)} multiline />
        <CopyField label="curl" value={firstRequestCurl(origin)} multiline />
        <p className="text-sm leading-relaxed text-muted-foreground">
          可用入口：<code className="rounded bg-muted px-1.5 py-0.5 font-mono text-xs text-foreground">POST /v1/chat/completions</code>、
          <code className="rounded bg-muted px-1.5 py-0.5 font-mono text-xs text-foreground">POST /v1/responses</code>、
          <code className="rounded bg-muted px-1.5 py-0.5 font-mono text-xs text-foreground">GET /v1/models</code>。查询额度走不计费的{' '}
          <code className="rounded bg-muted px-1.5 py-0.5 font-mono text-xs text-foreground">GET /v1/usage</code>
          。没有独立的 <code className="rounded bg-muted px-1.5 py-0.5 font-mono text-xs text-foreground">/v1/cursor</code> 或 Gemini{' '}
          <code className="rounded bg-muted px-1.5 py-0.5 font-mono text-xs text-foreground">/v1beta</code> 路径。
        </p>
      </div>
    </article>
  )
}

function ClaudeClient() {
  const origin = gatewayOrigin()
  return (
    <article className="max-w-4xl space-y-6">
      <div className="space-y-6">
        <CopyField label="环境变量" value={anthropicEnv(origin)} multiline />
        <CopyField
          label="实际请求"
          value={`POST ${origin}/v1/messages\nAuthorization: Bearer ${KEY_PREFIX}xxxxxxxx`}
          multiline
        />
        <p className="text-sm leading-relaxed text-muted-foreground">
          {PRODUCT_NAME} 在同一主机提供 <code className="rounded bg-muted px-1.5 py-0.5 font-mono text-xs text-foreground">POST /v1/messages</code>
          （以及不计费的 <code className="rounded bg-muted px-1.5 py-0.5 font-mono text-xs text-foreground">POST /v1/messages/count_tokens</code>
          ）。查询额度走不计费的 <code className="rounded bg-muted px-1.5 py-0.5 font-mono text-xs text-foreground">GET /v1/usage</code>
          ，用同一把 <code className="rounded bg-muted px-1.5 py-0.5 font-mono text-xs text-foreground">ANTHROPIC_AUTH_TOKEN</code> 即可。Anthropic SDK
          会把 <code className="rounded bg-muted px-1.5 py-0.5 font-mono text-xs text-foreground">ANTHROPIC_BASE_URL</code> 与{' '}
          <code className="rounded bg-muted px-1.5 py-0.5 font-mono text-xs text-foreground">/v1/messages</code> 拼在一起，所以这里填{' '}
          <code className="rounded bg-muted px-1.5 py-0.5 font-mono text-xs text-foreground">{origin || '{origin}'}</code>，不要写成{' '}
          <code className="rounded bg-muted px-1.5 py-0.5 font-mono text-xs text-foreground">/v1</code>。
        </p>
      </div>
    </article>
  )
}

function CodexClient() {
  const origin = gatewayOrigin()
  return (
    <article className="max-w-4xl space-y-6">
      <div className="space-y-6">
        <CopyField label="环境变量" value={openaiEnv(origin)} multiline />
        <p className="text-sm leading-relaxed text-muted-foreground">
          将 <code className="rounded bg-muted px-1.5 py-0.5 font-mono text-xs text-foreground">OPENAI_BASE_URL</code> 设为{' '}
          <code className="rounded bg-muted px-1.5 py-0.5 font-mono text-xs text-foreground">{openaiBaseUrl(origin) || '{origin}/v1'}</code>，
          <code className="rounded bg-muted px-1.5 py-0.5 font-mono text-xs text-foreground">OPENAI_API_KEY</code> 填 {KEY_PREFIX} 密钥。
          本网关提供 <code className="rounded bg-muted px-1.5 py-0.5 font-mono text-xs text-foreground">POST /v1/responses</code> 与{' '}
          <code className="rounded bg-muted px-1.5 py-0.5 font-mono text-xs text-foreground">POST /v1/chat/completions</code>，没有单独的 Codex 路径。
        </p>
      </div>
    </article>
  )
}

function CursorClient() {
  const origin = gatewayOrigin()
  return (
    <article className="max-w-4xl space-y-6">
      <div className="space-y-6">
        <CopyField label="OpenAI API Base URL" value={openaiBaseUrl(origin)} />
        <CopyField label="API Key" value={`${KEY_PREFIX}xxxxxxxx`} />
        <p className="text-sm leading-relaxed text-muted-foreground">
          在 Cursor Settings → Models 中启用 OpenAI API Key，Base URL 填{' '}
          <code className="rounded bg-muted px-1.5 py-0.5 font-mono text-xs text-foreground">{openaiBaseUrl(origin) || '{origin}/v1'}</code>
          。Cursor 会先请求 <code className="rounded bg-muted px-1.5 py-0.5 font-mono text-xs text-foreground">GET /v1/models</code> 拉模型列表。
          没有 <code className="rounded bg-muted px-1.5 py-0.5 font-mono text-xs text-foreground">/v1/cursor</code> 专用入口。
        </p>
      </div>
    </article>
  )
}
