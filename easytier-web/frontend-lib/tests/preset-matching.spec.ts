import { describe, expect, it } from 'vitest'
import {
  DEFAULT_NETWORK_CONFIG,
  type NetworkConfig,
  type PresetSummary,
  networkConfigMatchesPreset,
  findMatchingPreset,
} from '../src/types/network'

function preset(name: string, netName: string, secret: string): PresetSummary {
  const cfg = DEFAULT_NETWORK_CONFIG()
  cfg.network_name = netName
  cfg.network_secret = secret
  return {
    id: 1,
    name,
    network_config: cfg,
    create_time: '',
    update_time: '',
  }
}

describe('preset network matching', () => {
  it('matches when name and secret are equal', () => {
    const p = preset('office', 'net-a', 'sec-a')
    const cfg: NetworkConfig = {
      ...DEFAULT_NETWORK_CONFIG(),
      network_name: 'net-a',
      network_secret: 'sec-a',
    }
    expect(networkConfigMatchesPreset(cfg, p)).toBe(true)
  })

  it('does not match when name differs', () => {
    const p = preset('office', 'net-a', 'sec-a')
    const cfg: NetworkConfig = {
      ...DEFAULT_NETWORK_CONFIG(),
      network_name: 'net-b',
      network_secret: 'sec-a',
    }
    expect(networkConfigMatchesPreset(cfg, p)).toBe(false)
  })

  it('does not match when secret differs', () => {
    const p = preset('office', 'net-a', 'sec-a')
    const cfg: NetworkConfig = {
      ...DEFAULT_NETWORK_CONFIG(),
      network_name: 'net-a',
      network_secret: 'other',
    }
    expect(networkConfigMatchesPreset(cfg, p)).toBe(false)
  })

  it('treats empty and undefined secret as equal', () => {
    const p = preset('open', 'net-x', '')
    const empty: NetworkConfig = {
      ...DEFAULT_NETWORK_CONFIG(),
      network_name: 'net-x',
      network_secret: '',
    }
    const none: NetworkConfig = {
      ...DEFAULT_NETWORK_CONFIG(),
      network_name: 'net-x',
    }
    expect(networkConfigMatchesPreset(empty, p)).toBe(true)
    expect(networkConfigMatchesPreset(none, p)).toBe(true)
  })

  it('returns undefined when no preset matches', () => {
    const cfg: NetworkConfig = {
      ...DEFAULT_NETWORK_CONFIG(),
      network_name: 'net-a',
      network_secret: 'sec-a',
    }
    expect(findMatchingPreset(cfg, [preset('x', 'net-b', 'sec-b')])).toBeUndefined()
  })

  it('returns the matching preset', () => {
    const cfg: NetworkConfig = {
      ...DEFAULT_NETWORK_CONFIG(),
      network_name: 'net-a',
      network_secret: 'sec-a',
    }
    const p = preset('office', 'net-a', 'sec-a')
    expect(findMatchingPreset(cfg, [preset('x', 'net-b', 'sec-b'), p])?.id).toBe(p.id)
  })
})
