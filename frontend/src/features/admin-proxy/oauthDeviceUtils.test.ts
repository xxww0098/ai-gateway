import { describe, expect, it } from 'vitest'
import {
  isDeviceFlowResponse,
  kiroStartBody,
  parseDeviceAuthStart,
  parseImportJson,
} from './oauthDeviceUtils'

describe('parseDeviceAuthStart', () => {
  it('reads user_code and verification URIs', () => {
    const parsed = parseDeviceAuthStart({
      state: 'st-1',
      user_code: 'WDJB-MJHT',
      verification_uri: 'https://auth.example/device',
      verification_uri_complete: 'https://auth.example/device?user_code=WDJB-MJHT',
      expires_in: 900,
      interval: 5,
      url: 'https://auth.example/device?user_code=WDJB-MJHT',
    })
    expect(parsed).not.toBeNull()
    expect(parsed?.userCode).toBe('WDJB-MJHT')
    expect(parsed?.state).toBe('st-1')
    expect(parsed?.verificationUri).toBe('https://auth.example/device')
    expect(parsed?.interval).toBe(5)
    expect(isDeviceFlowResponse({ user_code: 'ABC' })).toBe(true)
  })

  it('is not a device response without user_code', () => {
    expect(parseDeviceAuthStart({ url: 'https://example/auth', state: 's' })).toBeNull()
    expect(isDeviceFlowResponse({ auth_url: 'https://example/auth' })).toBe(false)
    expect(parseDeviceAuthStart(null)).toBeNull()
  })
})

describe('parseImportJson', () => {
  it('accepts snake_case access_token', () => {
    const result = parseImportJson('{"access_token":"at-1"}')
    expect(result.ok).toBe(true)
  })

  it('accepts camelCase accessToken', () => {
    const result = parseImportJson('{"accessToken":"at-2"}')
    expect(result.ok).toBe(true)
  })

  it('rejects empty or non-object JSON', () => {
    expect(parseImportJson('').ok).toBe(false)
    expect(parseImportJson('[]').ok).toBe(false)
    expect(parseImportJson('{').ok).toBe(false)
    expect(parseImportJson('{"refresh_token":"rt"}').ok).toBe(false)
  })
})

describe('kiroStartBody', () => {
  it('sends import token as-is', () => {
    expect(kiroStartBody({ method: 'import', token: { access_token: 'at' } })).toEqual({
      method: 'import',
      token: { access_token: 'at' },
    })
  })

  it('includes IDC start URL and region', () => {
    expect(
      kiroStartBody({
        method: 'idc',
        startUrl: 'https://d-example.awsapps.com/start',
        region: 'us-west-2',
      }),
    ).toEqual({
      method: 'idc',
      start_url: 'https://d-example.awsapps.com/start',
      region: 'us-west-2',
    })
  })
})
