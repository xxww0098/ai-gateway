import React, { useState } from 'react'
import { useNavigate, Link } from 'react-router-dom'
import { useLogin } from '@/features/auth/hooks'
import { Mail, Lock, ArrowRight, Loader2, Eye, EyeOff } from 'lucide-react'

export default function Login() {
  const [email, setEmail] = useState('')
  const [password, setPassword] = useState('')
  const [showPassword, setShowPassword] = useState(false)
  const navigate = useNavigate()
  const loginMutation = useLogin()

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault()
    loginMutation.mutate(
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
        <h1 className="text-xl font-bold tracking-tight text-foreground">欢迎回来</h1>
        <p className="text-sm text-muted-foreground">登录以管理密钥、用量与余额</p>
      </div>

      <form onSubmit={handleSubmit} className="space-y-5" noValidate>
        <div className="space-y-1">
          <label htmlFor="login-email" className="input-label">邮箱</label>
          <div className="relative">
            <div className="pointer-events-none absolute inset-y-0 left-0 flex items-center pl-3 text-muted-foreground">
              <Mail className="h-5 w-5" />
            </div>
            <input
              id="login-email"
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
          <label htmlFor="login-password" className="input-label">密码</label>
          <div className="relative">
            <div className="pointer-events-none absolute inset-y-0 left-0 flex items-center pl-3 text-muted-foreground">
              <Lock className="h-5 w-5" />
            </div>
            <input
              id="login-password"
              type={showPassword ? "text" : "password"}
              autoComplete="current-password"
              className="input pl-10 pr-10"
              placeholder="••••••••"
              value={password}
              onChange={e => setPassword(e.target.value)}
              required
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

        <button
          type="submit"
          disabled={loginMutation.isPending || !email || !password}
          className="btn btn-primary mt-2 w-full"
        >
          {loginMutation.isPending ? (
            <><Loader2 className="mr-2 h-5 w-5 animate-spin" /> 验证中...</>
          ) : (
            <>登录 <ArrowRight className="ml-1 h-4 w-4" /></>
          )}
        </button>
      </form>

      <div className="mt-6 text-center text-sm text-muted-foreground">
        还没有账户？{' '}
        <Link to="/register" className="font-semibold text-primary hover:underline">
          免费注册
        </Link>
      </div>
    </>
  )
}
