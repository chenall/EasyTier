import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { formatRelativeTime } from '../src/modules/utils'

describe('formatRelativeTime', () => {
    beforeEach(() => {
        vi.useFakeTimers()
        vi.setSystemTime(new Date('2026-07-28T12:00:00Z'))
    })

    afterEach(() => {
        vi.useRealTimers()
    })

    it('returns empty string for missing or empty input', () => {
        expect(formatRelativeTime(undefined)).toBe('')
        expect(formatRelativeTime(null)).toBe('')
        expect(formatRelativeTime('')).toBe('')
    })

    it('falls back to the raw string for unparseable input', () => {
        expect(formatRelativeTime('not-a-date')).toBe('not-a-date')
    })

    it('formats past times as "x ago"', () => {
        expect(formatRelativeTime('2026-07-28T11:55:00Z', 'en')).toBe('5 minutes ago')
        expect(formatRelativeTime('2026-07-28T10:00:00Z', 'en')).toBe('2 hours ago')
        expect(formatRelativeTime('2026-07-25T12:00:00Z', 'en')).toBe('3 days ago')
    })

    it('formats future times as "in x"', () => {
        expect(formatRelativeTime('2026-07-28T12:10:00Z', 'en')).toBe('in 10 minutes')
    })

    it('formats sub-minute differences in seconds', () => {
        expect(formatRelativeTime('2026-07-28T11:59:30Z', 'en')).toBe('30 seconds ago')
    })
})
