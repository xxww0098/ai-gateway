import { describe, expect, it } from 'vitest'
import { apiKeysFromQueryData } from './hooks'

describe('apiKeysFromQueryData', () => {
  it('reads the items envelope the keys page caches', () => {
    const keys = apiKeysFromQueryData({
      items: [{ id: 1, name: 'dev' }],
      page: 1,
      page_size: 20,
      total: 1,
      total_pages: 1,
    })
    expect(keys).toEqual([{ id: 1, name: 'dev' }])
  })

  it('keeps a bare array', () => {
    expect(apiKeysFromQueryData([{ id: 2, name: 'raw' }])).toEqual([{ id: 2, name: 'raw' }])
  })

  it('does not treat a missing items list as keys', () => {
    expect(apiKeysFromQueryData({ total: 3 })).toEqual([])
    expect(apiKeysFromQueryData(null)).toEqual([])
  })
})
