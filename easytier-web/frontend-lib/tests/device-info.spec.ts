import { describe, expect, it } from 'vitest'
import { buildDeviceInfo } from '../src/modules/utils'

describe('buildDeviceInfo', () => {
  it('carries alias, online, last_seen_at and tags from the backend item', () => {
    const raw = {
      client_url: 'tcp://1.2.3.4:1234',
      info: {
        hostname: 'host-a',
        easytier_version: '2.0.0',
        machine_id: { part1: 1, part2: 2, part3: 3, part4: 4 },
        running_network_instances: [],
      },
      location: { country: 'CN', city: 'BJ', region: undefined },
      alias: 'My Laptop',
      online: true,
      last_seen_at: '2026-07-28T00:00:00+08:00',
      tags: ['prod', 'beijing'],
    }
    const info = buildDeviceInfo(raw)
    expect(info.alias).toBe('My Laptop')
    expect(info.online).toBe(true)
    expect(info.last_seen_at).toBe('2026-07-28T00:00:00+08:00')
    expect(info.tags).toEqual(['prod', 'beijing'])
    expect(info.hostname).toBe('host-a')
    expect(info.easytier_version).toBe('2.0.0')
  })

  it('defaults new fields for offline devices without heartbeat info', () => {
    const raw = { alias: '', online: false, last_seen_at: '', tags: [] }
    const info = buildDeviceInfo(raw)
    expect(info.online).toBe(false)
    expect(info.tags).toEqual([])
    expect(info.alias).toBe('')
    expect(info.last_seen_at).toBe('')
  })

  it('surfaces registry-sourced identity for offline devices (no live info)', () => {
    // Offline device: the backend carries machine_id/hostname/easytier_version
    // from the persisted registry row, NOT from a live heartbeat `info`.
    const raw = {
      machine_id: '11111111-2222-3333-4444-555555555555',
      hostname: 'edge-gw-01',
      easytier_version: '2.6.4',
      alias: 'Gateway',
      online: false,
      last_seen_at: '2026-07-28T00:00:00+08:00',
      tags: ['prod'],
    }
    const info = buildDeviceInfo(raw)
    expect(info.machine_id).toBe('11111111-2222-3333-4444-555555555555')
    expect(info.hostname).toBe('edge-gw-01')
    expect(info.easytier_version).toBe('2.6.4')
    expect(info.online).toBe(false)
    // title fallback (alias || hostname) must not be empty for offline devices
    expect(info.alias || info.hostname).toBe('Gateway')
  })
})
