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

function PageTitle({ title, subtitle }: { title: string; subtitle: string }) {
  return (
    <div className="mb-8 space-y-3">
      <h1 className="text-3xl font-extrabold tracking-tight text-gray-900 dark:text-white sm:text-4xl">
        {title}
      </h1>
      <p className="max-w-2xl text-base leading-relaxed text-gray-600 dark:text-gray-400">
        {subtitle}
      </p>
    </div>
  )
}

function OriginNote({ origin }: { origin: string }) {
  return (
    <p className="text-sm text-gray-500 dark:text-dark-400">
      下方地址默认取当前站点 <code className="font-mono text-primary-600 dark:text-primary-400">{origin || 'window.location.origin'}</code>
      。生产环境请换成你实际部署 {PRODUCT_NAME} 的主机。
    </p>
  )
}

function Overview() {
  const origin = gatewayOrigin()
  const openai = openaiBaseUrl(origin)
  const anthropic = anthropicBaseUrl(origin)

  return (
    <article>
      <PageTitle
        title="接入指南"
        subtitle={`把客户端指向本网关，一把 ${KEY_PREFIX} 密钥调用已开通的模型`}
      />

      <section className="mb-10 space-y-4">
        <h2 className="text-lg font-bold text-gray-900 dark:text-white">网关地址</h2>
        <OriginNote origin={origin} />
        <div className="grid gap-4 md:grid-cols-2">
          <div className="glass-card p-5">
            <div className="mb-3 flex items-center gap-2 text-sm font-semibold text-gray-900 dark:text-white">
              <Server className="h-4 w-4 text-primary-500" />
              OpenAI 兼容
            </div>
            <CopyField label="Base URL" value={openai} />
            <p className="mt-3 text-xs leading-5 text-gray-500 dark:text-dark-400">
              对话走 <code className="font-mono">POST /v1/chat/completions</code>
              ，也提供 <code className="font-mono">POST /v1/responses</code> 与{' '}
              <code className="font-mono">GET /v1/models</code>。鉴权：
              <code className="font-mono"> Authorization: Bearer {KEY_PREFIX}...</code>
            </p>
          </div>
          <div className="glass-card p-5">
            <div className="mb-3 flex items-center gap-2 text-sm font-semibold text-gray-900 dark:text-white">
              <Server className="h-4 w-4 text-primary-500" />
              Anthropic / Claude
            </div>
            <CopyField label="Base URL" value={anthropic} />
            <p className="mt-3 text-xs leading-5 text-gray-500 dark:text-dark-400">
              与 OpenAI 同一主机。Claude / Anthropic SDK 会自行拼接{' '}
              <code className="font-mono">/v1/messages</code>，因此 Base URL 填裸{' '}
              <code className="font-mono">{'{origin}'}</code>，不要再加{' '}
              <code className="font-mono">/v1</code>。租户鉴权同样是 Bearer{' '}
              <code className="font-mono">{KEY_PREFIX}...</code>
              （请用 <code className="font-mono">ANTHROPIC_AUTH_TOKEN</code>）。
            </p>
          </div>
        </div>
      </section>

      <section className="mb-10 space-y-4">
        <h2 className="text-lg font-bold text-gray-900 dark:text-white">三步接入</h2>
        <ol className="grid gap-4 md:grid-cols-3">
          <li className="glass-card p-5">
            <div className="mb-3 flex h-9 w-9 items-center justify-center rounded-xl bg-primary-50 text-primary-600 dark:bg-primary-900/30 dark:text-primary-300">
              <LogIn className="h-4 w-4" />
            </div>
            <div className="text-xs font-semibold uppercase tracking-wider text-gray-400">01</div>
            <h3 className="mt-1 font-semibold text-gray-900 dark:text-white">注册 / 登录</h3>
            <p className="mt-2 text-sm text-gray-600 dark:text-gray-400">
              在本站创建账号并登录控制台。
            </p>
          </li>
          <li className="glass-card p-5">
            <div className="mb-3 flex h-9 w-9 items-center justify-center rounded-xl bg-primary-50 text-primary-600 dark:bg-primary-900/30 dark:text-primary-300">
              <KeyRound className="h-4 w-4" />
            </div>
            <div className="text-xs font-semibold uppercase tracking-wider text-gray-400">02</div>
            <h3 className="mt-1 font-semibold text-gray-900 dark:text-white">创建密钥</h3>
            <p className="mt-2 text-sm text-gray-600 dark:text-gray-400">
              打开 <code className="font-mono">/keys</code>，生成一把 {KEY_PREFIX} 前缀的 API Key。
            </p>
          </li>
          <li className="glass-card p-5">
            <div className="mb-3 flex h-9 w-9 items-center justify-center rounded-xl bg-primary-50 text-primary-600 dark:bg-primary-900/30 dark:text-primary-300">
              <Send className="h-4 w-4" />
            </div>
            <div className="text-xs font-semibold uppercase tracking-wider text-gray-400">03</div>
            <h3 className="mt-1 font-semibold text-gray-900 dark:text-white">发第一请求</h3>
            <p className="mt-2 text-sm text-gray-600 dark:text-gray-400">
              用 Bearer 把密钥带到 <code className="font-mono">/v1/chat/completions</code>。余额不足返回 402。
            </p>
          </li>
        </ol>
        <CopyField label="curl 示例" value={firstRequestCurl(origin)} multiline />
        <div className="pt-2">
          <KeysCta />
        </div>
      </section>
    </article>
  )
}

function Quickstart() {
  const origin = gatewayOrigin()
  return (
    <article>
      <PageTitle
        title="快速接入"
        subtitle="把环境变量指到本网关，即可用现有 OpenAI / Anthropic 客户端调用已开通模型。"
      />
      <OriginNote origin={origin} />
      <div className="mt-6 space-y-6">
        <div className="glass-card space-y-4 p-5">
          <h2 className="text-base font-bold text-gray-900 dark:text-white">OpenAI 兼容</h2>
          <CopyField label="环境变量" value={openaiEnv(origin)} multiline />
          <p className="text-sm text-gray-600 dark:text-gray-400">
            SDK 与 Cursor、Codex CLI 等都把 Base URL 设为{' '}
            <code className="font-mono">{openaiBaseUrl(origin) || '{origin}/v1'}</code>
            ，密钥填 <code className="font-mono">{KEY_PREFIX}...</code>。
          </p>
        </div>
        <div className="glass-card space-y-4 p-5">
          <h2 className="text-base font-bold text-gray-900 dark:text-white">Anthropic / Claude Code</h2>
          <CopyField label="环境变量" value={anthropicEnv(origin)} multiline />
          <p className="text-sm text-gray-600 dark:text-gray-400">
            Base URL 是裸主机（SDK 会请求 <code className="font-mono">/v1/messages</code>
            ）。本网关只认 <code className="font-mono">Authorization: Bearer</code>，请设置{' '}
            <code className="font-mono">ANTHROPIC_AUTH_TOKEN</code>。
          </p>
        </div>
        <CopyField label="第一请求" value={firstRequestCurl(origin)} multiline />
        <p className="text-sm text-gray-500 dark:text-dark-400">
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
    <article>
      <PageTitle
        title="OpenAI SDK / curl"
        subtitle="官方 OpenAI SDK、HTTP 客户端以及任何 OpenAI 兼容工具，Base URL 设为 /v1 即可。"
      />
      <div className="space-y-6">
        <CopyField label="环境变量" value={openaiEnv(origin)} multiline />
        <CopyField label="Python" value={pythonOpenaiSnippet(origin)} multiline />
        <CopyField label="curl" value={firstRequestCurl(origin)} multiline />
        <p className="text-sm text-gray-600 dark:text-gray-400">
          可用入口：<code className="font-mono">POST /v1/chat/completions</code>、
          <code className="font-mono">POST /v1/responses</code>、
          <code className="font-mono">GET /v1/models</code>。没有独立的{' '}
          <code className="font-mono">/v1/cursor</code> 或 Gemini{' '}
          <code className="font-mono">/v1beta</code> 路径。
        </p>
      </div>
    </article>
  )
}

function ClaudeClient() {
  const origin = gatewayOrigin()
  return (
    <article>
      <PageTitle
        title="Claude Code"
        subtitle="Anthropic Messages 与 OpenAI 共用同一主机。Claude Code 把 Base URL 指到本站即可。"
      />
      <div className="space-y-6">
        <CopyField label="环境变量" value={anthropicEnv(origin)} multiline />
        <CopyField
          label="实际请求"
          value={`POST ${origin}/v1/messages\nAuthorization: Bearer ${KEY_PREFIX}xxxxxxxx`}
          multiline
        />
        <p className="text-sm leading-6 text-gray-600 dark:text-gray-400">
          {PRODUCT_NAME} 在同一主机提供 <code className="font-mono">POST /v1/messages</code>
          （以及不计费的 <code className="font-mono">POST /v1/messages/count_tokens</code>
          ）。Anthropic SDK 会把 <code className="font-mono">ANTHROPIC_BASE_URL</code> 与{' '}
          <code className="font-mono">/v1/messages</code> 拼在一起，所以这里填{' '}
          <code className="font-mono">{origin || '{origin}'}</code>，不要写成{' '}
          <code className="font-mono">/v1</code>。
        </p>
      </div>
    </article>
  )
}

function CodexClient() {
  const origin = gatewayOrigin()
  return (
    <article>
      <PageTitle
        title="Codex CLI"
        subtitle="按 OpenAI 兼容客户端配置。Codex 使用 /v1，需要 Responses 时走本网关的 /v1/responses。"
      />
      <div className="space-y-6">
        <CopyField label="环境变量" value={openaiEnv(origin)} multiline />
        <p className="text-sm leading-6 text-gray-600 dark:text-gray-400">
          将 <code className="font-mono">OPENAI_BASE_URL</code> 设为{' '}
          <code className="font-mono">{openaiBaseUrl(origin) || '{origin}/v1'}</code>，
          <code className="font-mono">OPENAI_API_KEY</code> 填 {KEY_PREFIX} 密钥。
          本网关提供 <code className="font-mono">POST /v1/responses</code> 与{' '}
          <code className="font-mono">POST /v1/chat/completions</code>，没有单独的 Codex 路径。
        </p>
      </div>
    </article>
  )
}

function CursorClient() {
  const origin = gatewayOrigin()
  return (
    <article>
      <PageTitle
        title="Cursor"
        subtitle="把 Cursor 的 OpenAI 兼容接口指到本网关 /v1，用 agw- 密钥调用已开通模型。"
      />
      <div className="space-y-6">
        <CopyField label="OpenAI API Base URL" value={openaiBaseUrl(origin)} />
        <CopyField label="API Key" value={`${KEY_PREFIX}xxxxxxxx`} />
        <p className="text-sm leading-6 text-gray-600 dark:text-gray-400">
          在 Cursor Settings → Models 中启用 OpenAI API Key，Base URL 填{' '}
          <code className="font-mono">{openaiBaseUrl(origin) || '{origin}/v1'}</code>
          。Cursor 会先请求 <code className="font-mono">GET /v1/models</code> 拉模型列表。
          没有 <code className="font-mono">/v1/cursor</code> 专用入口。
        </p>
      </div>
    </article>
  )
}
