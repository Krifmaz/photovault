// PHOTOVAULT: real disk on the storage card - `GET /photovault/usage`
// (contracts/storage-usage.md) and what the sidebar's storage card shows from
// it: the disk that holds the photos ("438 GB of 500 GB used"), the
// account's own photos as a second line, and a warning from 90% full. The
// same rules as the mobile app's `storage_usage.dart`.

import { authenticatedRequestHeaders, ensureOk } from "ente-base/http";
import { apiURL } from "ente-base/origins";
import { z } from "zod";

export const storageUsagePath = "/photovault/usage";

const Disk = z.object({
    totalBytes: z.number(),
    usedBytes: z.number(),
    availableBytes: z.number(),
    usedPercent: z.number().optional(),
    nearlyFull: z.boolean().optional(),
});

/** `GET /photovault/usage`; unknown fields are ignored. */
export const StorageUsage = z.object({
    usedBytes: z.number(),
    userBytes: z.number().nullish(),
    quotaBytes: z.number().nullish(),
    disk: Disk.nullish(),
});

export type StorageUsage = z.infer<typeof StorageUsage>;

export const nearlyFullPercent = 90;

export interface StorageCardView {
    /** "438 GB of 500 GB used", or "7.4 GB used" without a disk. */
    headline: string;
    /** "Your photos: 7.4 GB" (with a quota: "Your photos: 7.4 GB of 50 GB"). */
    detail: string;
    /** Bar: the whole disk's fill, 0..1. */
    diskFraction: number;
    /** Bar: this account's share of the disk, 0..1. */
    userFraction: number;
    /** "Server disk is 93% full", or undefined below 90%. */
    warning: string | undefined;
}

/**
 * Bytes the way `df -h` and the rest of the app count them (1024-based,
 * labelled GB/TB): "512 MB", "7.4 GB", "1.8 TB".
 */
export const formatStorageBytes = (bytes: number): string => {
    const units = ["bytes", "KB", "MB", "GB", "TB", "PB"];
    if (bytes < 1024) return `${Math.max(0, Math.round(bytes))} bytes`;
    let v = bytes;
    let i = 0;
    while (v >= 1024 && i < units.length - 1) {
        v /= 1024;
        i++;
    }
    const text =
        v >= 100 || i <= 2
            ? String(Math.round(v))
            : v.toFixed(1).replace(/\.0$/, "");
    return `${text} ${units[i]}`;
};

const fraction = (part: number, whole: number) =>
    whole <= 0 ? 0 : Math.min(Math.max(part / whole, 0), 1);

export const storageCardView = (u: StorageUsage): StorageCardView => {
    const quota = u.quotaBytes && u.quotaBytes > 0 ? u.quotaBytes : undefined;
    const userBytes = u.userBytes ?? u.usedBytes;
    const detail =
        quota !== undefined
            ? `Your photos: ${formatStorageBytes(u.usedBytes)} of ${formatStorageBytes(quota)}`
            : `Your photos: ${formatStorageBytes(userBytes)}`;
    const disk = u.disk && u.disk.totalBytes > 0 ? u.disk : undefined;
    if (!disk) {
        return {
            headline:
                quota !== undefined
                    ? `${formatStorageBytes(u.usedBytes)} of ${formatStorageBytes(quota)} used`
                    : `${formatStorageBytes(u.usedBytes)} used`,
            detail,
            diskFraction:
                quota !== undefined ? fraction(u.usedBytes, quota) : 0,
            userFraction:
                quota !== undefined ? fraction(u.usedBytes, quota) : 0,
            warning: undefined,
        };
    }
    const denom = disk.usedBytes + disk.availableBytes;
    const percent =
        disk.usedPercent ?? (denom > 0 ? (disk.usedBytes / denom) * 100 : 0);
    const nearlyFull = disk.nearlyFull ?? percent >= nearlyFullPercent;
    return {
        headline: `${formatStorageBytes(disk.usedBytes)} of ${formatStorageBytes(disk.totalBytes)} used`,
        detail,
        diskFraction: fraction(disk.usedBytes, disk.totalBytes),
        userFraction: fraction(userBytes, disk.totalBytes),
        warning: nearlyFull
            ? `Server disk is ${Math.floor(percent)}% full`
            : undefined,
    };
};

/**
 * Fetch `/photovault/usage`; undefined when the server does not have it (a
 * stock Ente server answers 404) or the body does not parse.
 */
export const fetchStorageUsage = async (): Promise<
    StorageUsage | undefined
> => {
    const res = await fetch(await apiURL(storageUsagePath), {
        headers: await authenticatedRequestHeaders(),
    });
    if (res.status == 404) return undefined;
    ensureOk(res);
    const parsed = StorageUsage.safeParse(await res.json());
    return parsed.success ? parsed.data : undefined;
};
