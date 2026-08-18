import React, { useState } from 'react'
import { useNavigate, Link } from 'react-router-dom'
import { useRegister } from '@/features/auth/hooks'
import { errorMessage } from '@/shared/api/errors'
import { Mail, Lock, ArrowRight, Loader2, AlertCircle, Eye, EyeOff } from 'lucide-react'

export default function Register() {
  const [email, setEmail] = useState('')
  const [password, setPassword] = useState('')
  const [showPassword, setShowPassword] = useState(false)
  const [error, setError] = useState('')
  const navigate = useNavigate()
  const registerMutation = useRegister()

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault()
    setError('')
    registerMutation.mutate(
      { email, password },
      {
        // Registration already returns a valid token and creates an active
        // account, so log the user straight in instead of bouncing them back
        // to the login screen to re-enter the credentials they just typed.
        onSuccess: () => {
          navigate('/dashboard')
        },
        onError: (err) => {
          setError(errorMessage(err, '注册失败'))
        },
      }
    )
  }

  return (
    <>
      <div className="text-center mb-8">
        <h2 className="text-2xl font-bold text-gray-900 dark:text-white mb-2">创建新账户</h2>
        <p className="text-gray-500 dark:text-gray-400">创建您的开发者账户</p>
      </div>

      <form onSubmit={handleSubmit} className="space-y-5">
        {error && (
          <div className="flex items-center gap-2 p-3 rounded-xl bg-red-50 dark:bg-red-900/20 text-red-600 dark:text-red-400 border border-red-100 dark:border-red-900/50 animate-in fade-in slide-in-from-top-2">
            <AlertCircle className="w-5 h-5 flex-shrink-0" />
            <span className="text-sm font-medium">{error}</span>
          </div>
        )}

        <div className="space-y-1">
          <label htmlFor="register-email" className="input-label">邮箱</label>
          <div className="relative">
            <div className="absolute inset-y-0 left-0 pl-3 flex items-center pointer-events-none text-gray-400">
              <Mail className="h-5 w-5" />
            </div>
            <input
              id="register-email"
              type="email"
              autoComplete="email"
              className="input pl-10"
              placeholder="yours@example.com"
              value={email}
              onChange={e => setEmail(e.target.value)}
              required
            />
          </div>
        </div>

        <div className="space-y-1">
          <label htmlFor="register-password" className="input-label">设置密码</label>
          <div className="relative">
            <div className="absolute inset-y-0 left-0 pl-3 flex items-center pointer-events-none text-gray-400">
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
              className="absolute inset-y-0 right-0 pr-3 flex items-center text-gray-400 hover:text-gray-600 dark:hover:text-gray-300 transition-colors"
            >
              {showPassword ? <EyeOff className="h-5 w-5" /> : <Eye className="h-5 w-5" />}
            </button>
          </div>
        </div>

        <button
          type="submit"
          disabled={registerMutation.isPending || !email || !password}
          className="btn btn-primary w-full mt-2"
        >
          {registerMutation.isPending ? (
            <><Loader2 className="w-5 h-5 animate-spin mr-2" /> 注册中...</>
          ) : (
            <>立即注册 <ArrowRight className="w-4 h-4 ml-1" /></>
          )}
        </button>
      </form>

      <div className="mt-6 text-center text-sm text-gray-600 dark:text-gray-400">
        已有账户？{' '}
        <Link to="/login" className="font-semibold text-primary-600 hover:text-primary-500 transition-colors">
          直接登录
        </Link>
      </div>
    </>
  )
}
