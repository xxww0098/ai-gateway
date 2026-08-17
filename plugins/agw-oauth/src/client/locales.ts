/** AGW Oauth settings page copy. Nav label is always "AGW Oauth". */

const en = {
  nav: 'AGW Oauth',
  title: 'AGW Oauth',
  description: 'Connect to the AI-GateWay and sign in securely in the browser.',
  originLabel: 'Gateway URL',
  originPlaceholder: 'https://gw.example.com',
  loggedIn: 'Signed in · OAuth credentials available',
  loggedOut: 'Not signed in',
  login: 'Sign in',
  logout: 'Sign out',
  saving: 'Saving…',
  waiting: 'Finish signing in to AI-GateWay in the browser.',
  userCode: 'User code',
  openUrl: 'Verification URL',
  error: 'Something went wrong',
} as const

const zh = {
  nav: 'AGW Oauth',
  title: 'AGW Oauth',
  description: '连接 AI-GateWay 网关并通过浏览器安全登录。',
  originLabel: '网关地址',
  originPlaceholder: 'https://gw.example.com',
  loggedIn: '已登录 · OAuth 凭据可用',
  loggedOut: '未登录',
  login: '登录',
  logout: '退出登录',
  saving: '保存中…',
  waiting: '请在浏览器中完成 AI-GateWay 登录。',
  userCode: '用户码',
  openUrl: '验证地址',
  error: '出错了',
} as const

export type AgwKey = typeof en
export const NS = 'settings.agw-oauth'
export { en, zh }
