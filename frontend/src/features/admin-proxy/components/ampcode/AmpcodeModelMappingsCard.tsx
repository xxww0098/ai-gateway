import { useState } from 'react'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/shared/components/ui/card'
import { Input } from '@/shared/components/ui/input'
import { Switch } from '@/shared/components/ui/switch'
import { Button } from '@/shared/components/ui/button'
import { Badge } from '@/shared/components/ui/badge'
import { EmptyState } from '@/shared/components/EmptyState'
import {
  ArrowRight,
  ArrowUp,
  ArrowDown,
  Plus,
  Trash2,
  GitFork,
  ShieldCheck,
  Sparkles,
  Save,
  RotateCcw,
  Loader2,
  AlertCircle,
  CheckCircle2,
  SlidersHorizontal,
} from 'lucide-react'
import {
  isRegexValid,
  simulateModelMapping,
  validateAmpModelMappings,
  type AmpModelMapping,
} from '../../ampcodeConfig'
import { cn } from '@/shared/utils/utils'

interface AmpcodeModelMappingsCardProps {
  forceModelMappings: boolean
  onToggleForceMappings: (enabled: boolean) => Promise<void>
  togglingForce: boolean
  modelMappings: AmpModelMapping[]
  setModelMappings: React.Dispatch<React.SetStateAction<AmpModelMapping[]>>
  savedMappings: AmpModelMapping[]
  onSaveMappings: () => Promise<void>
  onDiscardMappings: () => void
  savingMappings: boolean
}

export function AmpcodeModelMappingsCard({
  forceModelMappings,
  onToggleForceMappings,
  togglingForce,
  modelMappings,
  setModelMappings,
  savedMappings,
  onSaveMappings,
  onDiscardMappings,
  savingMappings,
}: AmpcodeModelMappingsCardProps) {
  const [testModelInput, setTestModelInput] = useState('claude-opus-4-5-20251101')

  // Dirty state tracking
  const isDirty = JSON.stringify(modelMappings) !== JSON.stringify(savedMappings)

  // Simulation
  const simulationResult = simulateModelMapping(testModelInput, modelMappings)

  // Row update helpers
  const updateRow = (index: number, patch: Partial<AmpModelMapping>) => {
    setModelMappings(prev =>
      prev.map((item, i) => (i === index ? { ...item, ...patch } : item)),
    )
  }

  const addRow = () => {
    setModelMappings(prev => [...prev, { from: '', to: '', regex: false }])
  }

  const deleteRow = (index: number) => {
    setModelMappings(prev => prev.filter((_, i) => i !== index))
  }

  const moveRow = (index: number, direction: 'up' | 'down') => {
    setModelMappings(prev => {
      const next = [...prev]
      const targetIndex = direction === 'up' ? index - 1 : index + 1
      if (targetIndex < 0 || targetIndex >= next.length) return prev
      const temp = next[index]
      next[index] = next[targetIndex]
      next[targetIndex] = temp
      return next
    })
  }

  const applyPreset = (preset: AmpModelMapping) => {
    setModelMappings(prev => [...prev, preset])
  }

  const validationErrors = validateAmpModelMappings(modelMappings)

  return (
    <div className="space-y-6">
      {/* Policy Card */}
      <Card className="border-border/80 shadow-2xs">
        <CardHeader className="pb-4">
          <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-4">
            <div className="space-y-1">
              <CardTitle className="text-base sm:text-lg font-semibold flex items-center gap-2">
                <ShieldCheck className="h-5 w-5 text-primary" />
                <span>强制开启模型重定向 (Force Model Mappings)</span>
              </CardTitle>
              <CardDescription className="text-xs sm:text-sm">
                控制模型重定向策略是作为「主路由规则」全局优先执行，还是作为「故障备用兜底」在原模型无渠道时触发。
              </CardDescription>
            </div>
            <div className="flex items-center gap-3">
              {togglingForce ? (
                <Loader2 className="h-5 w-5 animate-spin text-muted-foreground" />
              ) : (
                <div className="flex items-center gap-2.5">
                  <Badge variant={forceModelMappings ? 'warning' : 'outline'} className="text-xs font-medium">
                    {forceModelMappings ? '全局强制重定向' : '故障旁路触发'}
                  </Badge>
                  <Switch
                    checked={forceModelMappings}
                    onCheckedChange={onToggleForceMappings}
                    className="cursor-pointer"
                  />
                </div>
              )}
            </div>
          </div>
        </CardHeader>
      </Card>

      {/* Live Rule Simulator Card */}
      <Card className="border-border/80 shadow-2xs bg-gradient-to-br from-card to-muted/20">
        <CardHeader className="pb-3">
          <CardTitle className="text-base font-semibold flex items-center gap-2">
            <Sparkles className="h-4.5 w-4.5 text-primary" />
            <span>模型重定向实测模拟器</span>
          </CardTitle>
          <CardDescription className="text-xs sm:text-sm">
            在应用生效前，输入待测试的模型名称实时预览匹配结果及目标重定向模型，确保正则与精确匹配规则准确无误。
          </CardDescription>
        </CardHeader>

        <CardContent className="space-y-3">
          <div className="flex flex-col sm:flex-row items-stretch sm:items-center gap-3">
            <div className="relative flex-1">
              <Input
                value={testModelInput}
                onChange={e => setTestModelInput(e.target.value)}
                placeholder="输入待测试的模型名称 (如 claude-opus-4-5-20251101 或 gpt-4o)"
                className="font-mono text-xs h-9 bg-background shadow-2xs"
              />
            </div>
            <Button
              type="button"
              variant="outline"
              size="sm"
              onClick={() => setTestModelInput('claude-opus-4-5-20251101')}
              className="h-9 text-xs shrink-0 cursor-pointer shadow-2xs"
            >
              填入示例
            </Button>
          </div>

          {/* Simulation Output Banner */}
          {testModelInput.trim() && (
            <div
              className={cn(
                'rounded-xl border p-3.5 text-xs shadow-2xs transition-all',
                simulationResult.matched
                  ? 'border-emerald-200 bg-emerald-50/60 text-emerald-950 dark:border-emerald-800/40 dark:bg-emerald-950/20 dark:text-emerald-200'
                  : 'border-border bg-muted/40 text-muted-foreground',
              )}
            >
              <div className="flex flex-wrap items-center justify-between gap-2">
                <div className="flex items-center gap-2 flex-wrap">
                  {simulationResult.matched ? (
                    <>
                      <Badge variant="success" className="gap-1 px-2">
                        <CheckCircle2 className="h-3.5 w-3.5" />
                        <span>命中规则 #{simulationResult.ruleIndex + 1}</span>
                      </Badge>
                      <span className="font-medium text-foreground">
                        {simulationResult.matchedRule?.regex ? '正则表达式模式' : '精确匹配模式'}
                      </span>
                    </>
                  ) : (
                    <Badge variant="outline" className="px-2 text-muted-foreground">
                      未命中任何规则
                    </Badge>
                  )}
                </div>

                <div className="flex items-center gap-1.5 font-mono text-xs">
                  <span className="text-muted-foreground">最终调度:</span>
                  <span className="font-bold text-foreground bg-background/80 px-2 py-0.5 rounded border border-current/20">
                    {simulationResult.targetModel}
                  </span>
                </div>
              </div>

              {simulationResult.matched && simulationResult.matchedRule && (
                <p className="mt-2 text-[11px] opacity-85 leading-relaxed font-mono">
                  匹配模式: <code className="bg-background/60 px-1 rounded">{simulationResult.matchedRule.from}</code>
                  {' → '}
                  重写规则: <code className="bg-background/60 px-1 rounded">{simulationResult.matchedRule.to}</code>
                </p>
              )}
            </div>
          )}
        </CardContent>
      </Card>

      {/* Rules Table Card */}
      <Card className="border-border/80 shadow-2xs">
        <CardHeader className="pb-4">
          <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-3">
            <div>
              <CardTitle className="text-base sm:text-lg font-semibold flex items-center gap-2">
                <GitFork className="h-5 w-5 text-primary" />
                <span>模型重定向规则列表 (Model Mappings)</span>
              </CardTitle>
              <CardDescription className="text-xs sm:text-sm mt-1">
                按从上至下的优先顺序依次评估。支持通配符或正则表达式动态捕获组（如 <code>$1</code> 替换）。
              </CardDescription>
            </div>

            <div className="flex items-center gap-2 shrink-0 flex-wrap">
              <Button
                variant="outline"
                size="sm"
                onClick={addRow}
                className="h-8.5 px-3 text-xs gap-1.5 cursor-pointer shadow-2xs"
              >
                <Plus className="h-3.5 w-3.5" />
                添加规则
              </Button>
            </div>
          </div>
        </CardHeader>

        <CardContent className="space-y-4">
          {/* Presets Quick Bar */}
          <div className="flex flex-wrap items-center gap-2 p-2.5 rounded-xl border border-border/70 bg-muted/20 text-xs">
            <span className="text-muted-foreground font-medium flex items-center gap-1">
              <SlidersHorizontal className="h-3.5 w-3.5 text-primary" />
              快速填入常见预设:
            </span>
            <Button
              type="button"
              variant="secondary"
              size="sm"
              className="h-7 text-[11px] px-2.5 cursor-pointer"
              onClick={() =>
                applyPreset({
                  from: 'claude-opus-4-5-20251101',
                  to: 'gemini-claude-sonnet-4-5',
                  regex: false,
                })
              }
            >
              Claude Opus 4.5 → Gemini
            </Button>
            <Button
              type="button"
              variant="secondary"
              size="sm"
              className="h-7 text-[11px] px-2.5 cursor-pointer"
              onClick={() =>
                applyPreset({
                  from: '^claude-(.*)$',
                  to: 'gemini-claude-$1',
                  regex: true,
                })
              }
            >
              ^claude-(.*) → gemini-claude-$1 (正则)
            </Button>
            <Button
              type="button"
              variant="secondary"
              size="sm"
              className="h-7 text-[11px] px-2.5 cursor-pointer"
              onClick={() =>
                applyPreset({
                  from: 'gpt-4o',
                  to: 'local-gpt-4o',
                  regex: false,
                })
              }
            >
              GPT-4o → Local
            </Button>
          </div>

          {/* Table / List */}
          {modelMappings.length === 0 ? (
            <EmptyState
              icon={GitFork}
              title="暂无模型映射规则"
              description="添加模型映射规则以实现自动容灾转移、模型重定向或别名分发。"
              tone="default"
              bordered
              action={{
                label: '添加首条映射规则',
                onClick: addRow,
              }}
            />
          ) : (
            <div className="space-y-2.5">
              {modelMappings.map((mapping, index) => {
                const regexCheck = mapping.regex ? isRegexValid(mapping.from) : { valid: true }

                return (
                  <div
                    key={index}
                    className="p-3 rounded-xl border border-border/70 bg-card shadow-2xs space-y-2 hover:border-border transition-colors"
                  >
                    <div className="grid gap-2.5 sm:grid-cols-[auto_1fr_auto_1fr_auto_auto] items-center">
                      {/* Priority Badge & Move buttons */}
                      <div className="flex items-center gap-1">
                        <span className="font-mono text-xs text-muted-foreground w-6 text-center font-bold">
                          #{index + 1}
                        </span>
                        <div className="flex flex-col">
                          <Button
                            type="button"
                            variant="ghost"
                            size="icon"
                            disabled={index === 0}
                            onClick={() => moveRow(index, 'up')}
                            className="h-5 w-5 text-muted-foreground hover:text-foreground"
                            title="上移（提高优先级）"
                          >
                            <ArrowUp className="h-3 w-3" />
                          </Button>
                          <Button
                            type="button"
                            variant="ghost"
                            size="icon"
                            disabled={index === modelMappings.length - 1}
                            onClick={() => moveRow(index, 'down')}
                            className="h-5 w-5 text-muted-foreground hover:text-foreground"
                            title="下移（降低优先级）"
                          >
                            <ArrowDown className="h-3 w-3" />
                          </Button>
                        </div>
                      </div>

                      {/* Source Model (From) */}
                      <div className="space-y-1">
                        <Input
                          className={cn(
                            'h-9 text-xs font-mono bg-background shadow-2xs',
                            !regexCheck.valid ? 'border-destructive focus-visible:ring-destructive' : '',
                          )}
                          value={mapping.from}
                          onChange={e => updateRow(index, { from: e.target.value })}
                          placeholder="源模型名称 (如 claude-opus-4-5-20251101)"
                        />
                      </div>

                      <ArrowRight className="h-4 w-4 text-muted-foreground hidden sm:block shrink-0" />

                      {/* Target Model (To) */}
                      <div className="space-y-1">
                        <Input
                          className="h-9 text-xs font-mono bg-background shadow-2xs"
                          value={mapping.to}
                          onChange={e => updateRow(index, { to: e.target.value })}
                          placeholder="重定向目标模型 (如 gemini-claude-sonnet-4-5)"
                        />
                      </div>

                      {/* Regex Switch */}
                      <div className="flex items-center gap-2 rounded-lg border border-border/70 px-3 py-1.5 bg-muted/30 shrink-0">
                        <Switch
                          checked={!!mapping.regex}
                          onCheckedChange={v => updateRow(index, { regex: v })}
                          className="cursor-pointer"
                        />
                        <span className="text-xs font-medium text-muted-foreground select-none">Regex</span>
                      </div>

                      {/* Delete button */}
                      <Button
                        type="button"
                        variant="dangerIcon"
                        size="icon"
                        className="h-9 w-9 text-destructive shrink-0"
                        onClick={() => deleteRow(index)}
                        title="删除该规则"
                      >
                        <Trash2 className="h-4 w-4" />
                      </Button>
                    </div>

                    {/* Regex Validation Error Banner */}
                    {!regexCheck.valid && (
                      <div className="flex items-center gap-1.5 text-[11px] text-destructive bg-destructive/10 px-2.5 py-1 rounded-md">
                        <AlertCircle className="h-3.5 w-3.5 shrink-0" />
                        <span>正则表达式语法无效: {regexCheck.error}</span>
                      </div>
                    )}
                  </div>
                )
              })}
            </div>
          )}

          {/* Validation Warnings */}
          {validationErrors.length > 0 && (
            <div className="rounded-xl border border-destructive/30 bg-destructive/10 p-3 text-xs text-destructive space-y-1">
              <div className="font-semibold flex items-center gap-1.5">
                <AlertCircle className="h-4 w-4 shrink-0" />
                <span>映射配置存在以下问题，请修正后再保存：</span>
              </div>
              <ul className="list-disc list-inside space-y-0.5 text-[11px] pl-1">
                {validationErrors.map((err, i) => (
                  <li key={i}>{err}</li>
                ))}
              </ul>
            </div>
          )}

          {/* Save & Reset Actions Bar */}
          <div className="flex flex-wrap items-center justify-between gap-3 pt-3 border-t border-border/60">
            <div className="text-xs text-muted-foreground">
              {isDirty ? (
                <span className="text-amber-600 dark:text-amber-400 font-medium">
                  ● 有未保存的规则修改
                </span>
              ) : (
                '映射规则与网关服务器保持同步'
              )}
            </div>

            <div className="flex items-center gap-2">
              {isDirty && (
                <Button
                  variant="outline"
                  size="sm"
                  className="h-8.5 px-3 text-xs gap-1.5 cursor-pointer shadow-2xs"
                  onClick={onDiscardMappings}
                  disabled={savingMappings}
                >
                  <RotateCcw className="h-3.5 w-3.5" />
                  放弃修改
                </Button>
              )}
              <Button
                size="sm"
                className="h-8.5 px-3.5 text-xs gap-1.5 cursor-pointer shadow-2xs"
                onClick={onSaveMappings}
                disabled={savingMappings || !isDirty || validationErrors.length > 0}
              >
                {savingMappings ? (
                  <Loader2 className="h-3.5 w-3.5 animate-spin" />
                ) : (
                  <Save className="h-3.5 w-3.5" />
                )}
                保存所有映射规则
              </Button>
            </div>
          </div>
        </CardContent>
      </Card>
    </div>
  )
}
