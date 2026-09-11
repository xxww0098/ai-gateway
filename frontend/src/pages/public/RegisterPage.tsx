import React, { useState } from 'react'
import { useNavigate, Link } from 'react-router-dom'
import { useRegister } from '@/features/auth/hooks'
import { Mail, Lock, ArrowRight, Loader2, Eye, EyeOff } from 'lucide-react'

export default function Register() {
  const [email, setEmail] = useState('')
  const [password, setPassword] = useState('')
  const [confirmPassword, setConfirmPassword] = useState('')
  const [showPassword, setShowPassword] = useState(false)
  const [showConfirmPassword, setShowConfirmPassword] = useState(false)
  const navigate = useNavigate()
  const registerMutation = useRegister()
  const passwordMismatch = confirmPassword.length > 0 && password !== confirmPassword
  const canSubmit =
    Boolean(email) &&
    Boolean(password) &&
    Boolean(confirmPassword) &&
    !passwordMismatch &&
    !registerMutation.isPending

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault()
    if (password !== confirmPassword) return
    registerMutation.mutate(
      { email, password },
      {
        onSuccess: () => {
          navigate('/dashboard')
        },
      }
    )
  }

  return (
    <>
      <div className="mb-6 space-y-1.5 text-center">
        <h1 className="text-xl font-bold tracking-tight text-foreground">创建账户</h1>
        <p className="text-sm text-muted-foreground">注册即可创建 agw- 密钥，接通多家上游模型</p>
      </div>

      <form onSubmit={handleSubmit} className="space-y-5" noValidate>
        <div className="space-y-1">
          <label htmlFor="register-email" className="input-label">邮箱</label>
          <div className="relative">
            <div className="pointer-events-none absolute inset-y-0 left-0 flex items-center pl-3 text-muted-foreground">
              <Mail className="h-5 w-5" />
            </div>
            <input
              id="register-email"
              type="email"
              autoComplete="email"
              className="input pl-10"
              placeholder="you@example.com"
              value={email}
              onChange={e => setEmail(e.target.value)}
              required
            />
          </div>
        </div>

        <div className="space-y-1">
          <label htmlFor="register-password" className="input-label">设置密码</label>
          <div className="relative">
            <div className="pointer-events-none absolute inset-y-0 left-0 flex items-center pl-3 text-muted-foreground">
              <Lock className="h-5 w-5" />
            </div>
            <input
              id="register-password"
              type={showPassword ? "text" : "password"}
              autoComplete="new-password"
              className="input pl-10 pr-10"
              placeholder="至少 8 位"
              value={password}
              onChange={e => setPassword(e.target.value)}
              required
              minLength={8}
            />
            <button
              type="button"
              aria-label={showPassword ? "隐藏密码" : "显示密码"}
              onClick={() => setShowPassword(!showPassword)}
              className="absolute inset-y-0 right-0 flex items-center pr-3 text-muted-foreground transition-colors hover:text-foreground"
            >
              {showPassword ? <EyeOff className="h-5 w-5" /> : <Eye className="h-5 w-5" />}
            </button>
          </div>
        </div>

        <div className="space-y-1">
          <label htmlFor="register-password-confirm" className="input-label">确认密码</label>
          <div className="relative">
            <div className="pointer-events-none absolute inset-y-0 left-0 flex items-center pl-3 text-muted-foreground">
              <Lock className="h-5 w-5" />
            </div>
            <input
              id="register-password-confirm"
              type={showConfirmPassword ? "text" : "password"}
              autoComplete="new-password"
              className={`input pl-10 pr-10${passwordMismatch ? ' input-error' : ''}`}
              placeholder="再次输入"
              value={confirmPassword}
              onChange={e => setConfirmPassword(e.target.value)}
              aria-invalid={passwordMismatch}
              aria-describedby={passwordMismatch ? 'register-password-mismatch' : undefined}
              required
              minLength={8}
            />
            <button
              type="button"
              aria-label={showConfirmPassword ? "隐藏确认密码" : "显示确认密码"}
              onClick={() => setShowConfirmPassword(!showConfirmPassword)}
              className="absolute inset-y-0 right-0 flex items-center pr-3 text-muted-foreground transition-colors hover:text-foreground"
            >
              {showConfirmPassword ? <EyeOff className="h-5 w-5" /> : <Eye className="h-5 w-5" />}
            </button>
          </div>
          {passwordMismatch && (
            <p id="register-password-mismatch" className="input-error-text" role="alert">
              两次密码不一致
            </p>
          )}
        </div>

        <button
          type="submit"
          disabled={!canSubmit}
          className="btn btn-primary mt-2 w-full"
        >
          {registerMutation.isPending ? (
            <><Loader2 className="mr-2 h-5 w-5 animate-spin" /> 注册中...</>
          ) : (
            <>立即注册 <ArrowRight className="ml-1 h-4 w-4" /></>
          )}
        </button>
      </form>

      <div className="mt-6 text-center text-sm text-muted-foreground">
        已有账户？{' '}
        <Link to="/login" className="font-semibold text-primary hover:underline">
          直接登录
        </Link>
      </div>
    </>
  )
}
