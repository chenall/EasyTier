import { IPv4, IPv6 } from 'ip-num/IPNumber'
import { Ipv4Addr, Ipv4Inet, Ipv6Addr } from '../types/network'

export function ipv4ToString(ip: Ipv4Addr | null | undefined) {
    if (!ip) {
        return ''
    }
    return IPv4.fromNumber(ip.addr ?? 0).toString()
}

export function ipv4InetToString(ip: Ipv4Inet | undefined) {
    if (ip?.address === undefined) {
        return 'undefined'
    }
    return `${ipv4ToString(ip.address)}/${ip.network_length ?? 0}`
}

export function ipv6ToString(ip: Ipv6Addr | null | undefined) {
    if (!ip) {
        return ''
    }
    return IPv6.fromBigInt(
        (BigInt(ip.part1 ?? 0) << BigInt(96))
        + (BigInt(ip.part2 ?? 0) << BigInt(64))
        + (BigInt(ip.part3 ?? 0) << BigInt(32))
        + BigInt(ip.part4 ?? 0),
    ).toString()
}

function toHexString(uint64: bigint, padding = 9): string {
    let hexString = uint64.toString(16);
    while (hexString.length < padding) {
        hexString = '0' + hexString;
    }
    return hexString;
}

function uint32ToUuid(part1: number, part2: number, part3: number, part4: number): string {
    // 将两个 uint64 转换为 16 进制字符串
    const part1Hex = toHexString(BigInt(part1), 8);
    const part2Hex = toHexString(BigInt(part2), 8);
    const part3Hex = toHexString(BigInt(part3), 8);
    const part4Hex = toHexString(BigInt(part4), 8);

    // 构造 UUID 格式字符串
    const uuid = `${part1Hex.substring(0, 8)}-${part2Hex.substring(0, 4)}-${part2Hex.substring(4, 8)}-${part3Hex.substring(0, 4)}-${part3Hex.substring(4, 8)}${part4Hex.substring(0, 12)}`;

    return uuid;
}

export interface UUID {
    part1?: number;
    part2?: number;
    part3?: number;
    part4?: number;
}

export function UuidToStr(uuid: UUID | null | undefined): string {
    if (!uuid) {
        return '';
    }
    return uint32ToUuid(uuid.part1 ?? 0, uuid.part2 ?? 0, uuid.part3 ?? 0, uuid.part4 ?? 0);
}

export function StrToUuid(uuid: string): UUID {
    const hex = uuid.replace(/-/g, '');
    if (!/^[0-9a-fA-F]{32}$/.test(hex)) {
        throw new Error(`Invalid UUID: ${uuid}`);
    }

    return {
        part1: Number.parseInt(hex.slice(0, 8), 16),
        part2: Number.parseInt(hex.slice(8, 16), 16),
        part3: Number.parseInt(hex.slice(16, 24), 16),
        part4: Number.parseInt(hex.slice(24, 32), 16),
    };
}

export interface Location {
    country: string | undefined;
    city: string | undefined;
    region: string | undefined;
}

export interface DeviceInfo {
    hostname: string;
    public_ip: string;
    running_network_count: number;
    report_time: string;
    easytier_version: string;
    running_network_instances?: Array<string>;
    machine_id: string;
    location: Location | undefined;
    alias?: string;
    online?: boolean;
    last_seen_at?: string;
    tags?: Array<string>;
}

export function buildDeviceInfo(device: any): DeviceInfo {
    const runningInstances = device.info?.running_network_instances ?? [];
    let dev_info: DeviceInfo = {
        // Prefer registry-sourced fields so OFFLINE devices (whose live `info`
        // is absent) still surface their last-known hostname/version and a stable
        // machine_id. Fall back to `info` for online-only extras.
        hostname: device.hostname ?? device.info?.hostname,
        easytier_version: device.easytier_version ?? device.info?.easytier_version,
        machine_id: device.machine_id ?? UuidToStr(device.info?.machine_id),
        public_ip: device.client_url,
        running_network_instances: runningInstances.map((instance: any) => UuidToStr(instance)),
        running_network_count: runningInstances.length,
        report_time: device.info?.report_time,
        location: device.location,
        alias: device.alias ?? '',
        online: device.online ?? false,
        last_seen_at: device.last_seen_at ?? '',
        tags: device.tags ?? [],
    };

    return dev_info;
}

/**
 * 将 ISO 时间戳格式化为相对时间，如「5 分钟前」「3 小时前」「2 天前」。
 * 离线设备展示 last_seen_at 时可直观体现「多久没心跳」。
 * 使用内置 Intl.RelativeTimeFormat，无第三方依赖；不传 locale 时使用运行环境默认（浏览器即用户语言）。
 */
export function formatRelativeTime(iso: string | undefined | null, locale?: string): string {
    if (!iso) {
        return '';
    }
    const then = new Date(iso).getTime();
    if (Number.isNaN(then)) {
        return iso;
    }
    const diffSec = Math.round((then - Date.now()) / 1000);
    const rtf = new Intl.RelativeTimeFormat(locale, { numeric: 'auto' });
    if (Math.abs(diffSec) < 60) {
        return rtf.format(Math.round(diffSec), 'second');
    }
    const min = Math.round(diffSec / 60);
    if (Math.abs(min) < 60) {
        return rtf.format(min, 'minute');
    }
    const hr = Math.round(diffSec / 3600);
    if (Math.abs(hr) < 24) {
        return rtf.format(hr, 'hour');
    }
    const day = Math.round(diffSec / 86400);
    return rtf.format(day, 'day');
}

// write a class to run a function periodically and can be stopped by calling stop(), use setTimeout to trigger the function
export class PeriodicTask {
    private interval: number;
    private task: (() => Promise<void>) | undefined;
    private timer: any;

    constructor(task: () => Promise<void>, interval: number) {
        this.interval = interval;
        this.task = task;
    }

    _runTaskHelper(nextInterval: number) {
        this.timer = setTimeout(async () => {
            if (this.task) {
                await this.task();
                this._runTaskHelper(this.interval);
            }
        }, nextInterval);
    }

    start() {
        this._runTaskHelper(0);
    }

    stop() {
        this.task = undefined;
        clearTimeout(this.timer);
    }
}
