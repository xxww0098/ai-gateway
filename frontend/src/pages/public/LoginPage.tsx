import React, { useState } from 'react'
import { useNavigate, Link } from 'react-router-dom'
import { useLogin } from '@/features/auth/hooks'
import { errorMessage } from '@/shared/api/errors'
import { Mail, Lock, ArrowRight, Loader2, AlertCircle, Eye, EyeOff } from 'lucide-react'

export default function Login() {
  const [email, setEmail] = useState('')
  const [password, setPassword] = useState('')
  const [showPassword, setShowPassword] = useState(false)
  const [error, setError] = useState('')
  const navigate = useNavigate()
  const loginMutation = useLogin()

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault()
    setError('')
    loginMutation.mutate(
      { email, password },
      {
        onSuccess: () => {
          navigate('/dashboard')
        },
        onError: (err) => {
          setError(errorMessage(err, '登录失败'))
        },
      }
    )
  }

  return (
    <>
      <div className="text-center mb-8">
        <h2 className="text-2xl font-bold text-gray-900 dark:text-white mb-2">欢迎回来</h2>
        <p className="text-gray-500 dark:text-gray-400">登入 CPA Gateway 计费管理平台</p>
      </div>

      <form onSubmit={handleSubmit} className="space-y-5">
        {error && (
          <div className="flex items-center gap-2 p-3 rounded-xl bg-red-50 dark:bg-red-900/20 text-red-600 dark:text-red-400 border border-red-100 dark:border-red-900/50 animate-in fade-in slide-in-from-top-2">
            <AlertCircle className="w-5 h-5 flex-shrink-0" />
            <span className="text-sm font-medium">{error}</span>
          </div>
        )}

        <div className="space-y-1">
          <label className="input-label">邮箱</label>
          <div className="relative">
            <div className="absolute inset-y-0 left-0 pl-3 flex items-center pointer-events-none text-gray-400">
              <Mail className="h-5 w-5" />
            </div>
            <input
              type="email"
              className="input pl-10"
              placeholder="admin@example.com"
              value={email}
              onChange={e => setEmail(e.target.value)}
              required
            />
          </div>
        </div>

        <div className="space-y-1">
          <label className="input-label">密码</label>
          <div className="relative">
            <div className="absolute inset-y-0 left-0 pl-3 flex items-center pointer-events-none text-gray-400">
              <Lock className="h-5 w-5" />
            </div>
            <input
              type={showPassword ? "text" : "password"}
              className="input pl-10 pr-10"
              placeholder="••••••••"
              value={password}
              onChange={e => setPassword(e.target.value)}
              required
            />
            <button
              type="button"
              onClick={() => setShowPassword(!showPassword)}
              className="absolute inset-y-0 right-0 pr-3 flex items-center text-gray-400 hover:text-gray-600 dark:hover:text-gray-300 transition-colors"
            >
              {showPassword ? <EyeOff className="h-5 w-5" /> : <Eye className="h-5 w-5" />}
            </button>
          </div>
        </div>

        <button 
          type="submit" 
          disabled={loginMutation.isPending || !email || !password}
          className="btn btn-primary w-full mt-2"
        >
          {loginMutation.isPending ? (
            <><Loader2 className="w-5 h-5 animate-spin mr-2" /> 验证中...</>
          ) : (
            <>登录系统 <ArrowRight className="w-4 h-4 ml-1" /></>
          )}
        </button>
      </form>

      <div className="mt-6 text-center text-sm text-gray-600 dark:text-gray-400">
        还没账户?{' '}
        <Link to="/register" className="font-semibold text-primary-600 hover:text-primary-500 transition-colors">
          免费注册
        </Link>
      </div>
    </>
  )
}
