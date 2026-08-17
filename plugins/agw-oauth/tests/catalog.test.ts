import assert from 'node:assert/strict'
import { test } from 'node:test'
import { parseModelsPayload, toResolvedModel } from '../lib/catalog.js'

const payload = {
  object: 'list',
  data: [
    {
      id: 'vision-thinker',
      object: 'model',
      context_length: 128000,
      max_output_tokens: 16384,
      input_modalities: ['text', 'image'],
      reasoning: {
        efforts: [
          { id: 'low', name: 'Low' },
          { id: 'high', name: 'High' },
        ],
        default_effort: 'high',
      },
    },
    {
      id: 'text-only',
      object: 'model',
      context_length: 8192,
      max_output_tokens: 2048,
      input_modalities: ['text'],
    },
  ],
}

test('vision model advertises image+text and thinking efforts from the catalog', () => {
  const [vision] = parseModelsPayload(payload)
  assert.ok(vision)
  assert.deepEqual(vision.inputModalities, ['text', 'image'])
  assert.ok(vision.reasoning)
  assert.ok(vision.reasoning.efforts.length > 0)
  const resolved = toResolvedModel('ai-gateway', vision)
  assert.equal(resolved.context?.contextWindow, 128000)
  assert.equal(resolved.defaultMaxTokens, 16384)
  assert.deepEqual(resolved.reasoning?.efforts.map(e => e.id), ['low', 'high'])
  assert.equal(resolved.reasoning?.defaultEffort, 'high')
})

test('text-only model does not advertise image and has no reasoning list', () => {
  const models = parseModelsPayload(payload)
  const text = models.find(m => m.id === 'text-only')
  assert.ok(text)
  assert.deepEqual(text.inputModalities, ['text'])
  assert.equal(text.inputModalities.includes('image'), false)
  assert.equal(text.reasoning, undefined)
  const resolved = toResolvedModel('ai-gateway', text)
  assert.equal(resolved.reasoning, undefined)
  assert.equal(resolved.context?.contextWindow, 8192)
  assert.equal(resolved.defaultMaxTokens, 2048)
})

test('token limits are catalog numbers, not hardcoded guesses', () => {
  const custom = parseModelsPayload({
    data: [{ id: 'odd', context_length: 3210, max_output_tokens: 99, input_modalities: ['text'] }],
  })
  assert.equal(custom[0]?.contextLength, 3210)
  assert.equal(custom[0]?.maxOutputTokens, 99)
  assert.notEqual(custom[0]?.contextLength, 128000)
  assert.notEqual(custom[0]?.maxOutputTokens, 16384)
})

test('missing capabilities stay empty instead of inventing thinking or vision', () => {
  const [plain] = parseModelsPayload({ data: [{ id: 'plain' }] })
  assert.ok(plain)
  assert.deepEqual(plain.inputModalities, [])
  assert.equal(plain.reasoning, undefined)
  assert.equal(plain.contextLength, undefined)
  assert.equal(plain.maxOutputTokens, undefined)
})
