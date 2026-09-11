import { Link } from 'react-router-dom'
import {
  Cpu,
  SearchX,
  Server,
  Network,
  Coins,
  ArrowRight,
  BookOpen,
  MessageSquarePlus,
  RotateCcw,
} from 'lucide-react'
import { adminRoutes } from '@/shared/routes/admin'
import { userRoutes } from '@/shared/routes/user'

interface ModelEmptyStateProps {
  isAdmin: boolean
  isFilterMiss?: boolean
  onClearFilter?: () => void
}

export function ModelEmptyState({
  isAdmin,
  isFilterMiss = false,
  onClearFilter,
}: ModelEmptyStateProps) {
  if (isFilterMiss) {
    return (
      <div className="flex flex-col items-center justify-center rounded-2xl border border-dashed border-border/90 bg-card/60 px-6 py-16 text-center shadow-2xs">
        <div className="flex h-14 w-14 items-center justify-center rounded-2xl bg-muted text-muted-foreground shadow-2xs">
          <SearchX className="h-7 w-7" />
        </div>
        <div className="mt-4 max-w-md space-y-1.5">
          <h3 className="text-base font-bold text-foreground">未找到匹配的模型</h3>
          <p className="text-sm text-muted-foreground leading-relaxed">
            当前搜索词或筛选条件下没有找到模型。请尝试调整关键词，或重置供应商与模态标签。
          </p>
        </div>
        {onClearFilter && (
          <button
            type="button"
            onClick={onClearFilter}
            className="mt-5 inline-flex items-center gap-1.5 rounded-xl border border-border bg-background px-4 py-2 text-xs font-semibold text-foreground shadow-2xs transition hover:bg-muted active:scale-[0.98]"
          >
            <RotateCcw className="h-3.5 w-3.5" />
            重置所有筛选
          </button>
        )}
      </div>
    )
  }

  return (
    <div className="rounded-2xl border border-border bg-card p-6 shadow-2xs sm:p-8 md:p-10">
      <div className="mx-auto max-w-3xl space-y-8">
        {/* Banner Title */}
        <div className="text-center space-y-3">
          <div className="inline-flex items-center gap-1.5 rounded-full border border-primary/20 bg-primary/10 px-3 py-1 text-xs font-semibold text-primary">
            <Cpu className="h-3.5 w-3.5" />
            <span>网关路由就绪 · 待接入上游模型</span>
          </div>
          <h2 className="text-xl font-bold tracking-tight text-foreground sm:text-2xl">
            还没有可用模型
          </h2>
          <p className="mx-auto max-w-xl text-sm text-muted-foreground leading-relaxed">
            AI-GateWay 采用独立上游渠道池与精准 Token 账本。上游渠道配置并启用后，支持的模型将自动同步至此向全体租户开放。
          </p>
        </div>

        {/* 3-Step Guided Architecture Cards */}
        <div className="grid gap-4 sm:grid-cols-3">
          <div className="relative rounded-xl border border-border/80 bg-muted/30 p-4 transition hover:border-border hover:bg-muted/50">
            <div className="mb-3 flex h-8 w-8 items-center justify-center rounded-lg bg-primary/15 text-xs font-bold text-primary">
              01
            </div>
            <h4 className="text-sm font-semibold text-foreground flex items-center gap-1.5">
              <Server className="h-4 w-4 text-primary" />
              接入上游渠道
            </h4>
            <p className="mt-1.5 text-xs text-muted-foreground leading-relaxed">
              在渠道管理接入 OpenAI、Claude、Gemini、DeepSeek 等官方凭证或中转聚合端点。
            </p>
          </div>

          <div className="relative rounded-xl border border-border/80 bg-muted/30 p-4 transition hover:border-border hover:bg-muted/50">
            <div className="mb-3 flex h-8 w-8 items-center justify-center rounded-lg bg-primary/15 text-xs font-bold text-primary">
              02
            </div>
            <h4 className="text-sm font-semibold text-foreground flex items-center gap-1.5">
              <Network className="h-4 w-4 text-primary" />
              开放与映射模型
            </h4>
            <p className="mt-1.5 text-xs text-muted-foreground leading-relaxed">
              选择允许对外开放的模型列表，自动补齐上下文规格元数据，并可配置自定义模型别名。
            </p>
          </div>

          <div className="relative rounded-xl border border-border/80 bg-muted/30 p-4 transition hover:border-border hover:bg-muted/50">
            <div className="mb-3 flex h-8 w-8 items-center justify-center rounded-lg bg-primary/15 text-xs font-bold text-primary">
              03
            </div>
            <h4 className="text-sm font-semibold text-foreground flex items-center gap-1.5">
              <Coins className="h-4 w-4 text-primary" />
              精算定价与调用
            </h4>
            <p className="mt-1.5 text-xs text-muted-foreground leading-relaxed">
              设定输入、输出、缓存与推理独立费率。租户使用 agw- 密钥即可在 /v1 接口中顺畅调用。
            </p>
          </div>
        </div>

        {/* Action Buttons */}
        <div className="flex flex-wrap items-center justify-center gap-3 pt-2">
          {isAdmin ? (
            <>
              <Link
                to={adminRoutes.channels}
                className="inline-flex items-center gap-2 rounded-xl bg-primary px-5 py-2.5 text-sm font-semibold text-primary-foreground shadow-sm transition hover:bg-primary/90 active:scale-[0.98]"
              >
                前往渠道管理配置
                <ArrowRight className="h-4 w-4" />
              </Link>
              <Link
                to="/docs"
                className="inline-flex items-center gap-1.5 rounded-xl border border-border bg-background px-4 py-2.5 text-sm font-semibold text-muted-foreground hover:text-foreground hover:bg-muted transition active:scale-[0.98]"
              >
                <BookOpen className="h-4 w-4" />
                查看接入文档
              </Link>
            </>
          ) : (
            <>
              <Link
                to={userRoutes.tickets}
                className="inline-flex items-center gap-2 rounded-xl bg-primary px-5 py-2.5 text-sm font-semibold text-primary-foreground shadow-sm transition hover:bg-primary/90 active:scale-[0.98]"
              >
                <MessageSquarePlus className="h-4 w-4" />
                提交工单咨询管理员
              </Link>
              <Link
                to="/docs"
                className="inline-flex items-center gap-1.5 rounded-xl border border-border bg-background px-4 py-2.5 text-sm font-semibold text-muted-foreground hover:text-foreground hover:bg-muted transition active:scale-[0.98]"
              >
                <BookOpen className="h-4 w-4" />
                阅读 API 接入文档
              </Link>
            </>
          )}
        </div>
      </div>
    </div>
  )
}
