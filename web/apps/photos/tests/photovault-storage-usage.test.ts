// PHOTOVAULT: real disk on the storage card (contracts/storage-usage.md).
import {
    StorageUsage,
    formatStorageBytes,
    storageCardView,
} from "ente-base/photovault/storage-usage";
import { describe, expect, it } from "vitest";

const gib = 1024 * 1024 * 1024;

const disk = (total: number, used: number, available: number) => ({
    totalBytes: total,
    usedBytes: used,
    availableBytes: available,
    usedPercent: (used / (used + available)) * 100,
    nearlyFull: (used / (used + available)) * 100 >= 90,
    source: "disk",
});

const usage = (o: {
    used?: number;
    user?: number;
    quota?: number | null;
    disk?: ReturnType<typeof disk> | null;
}) =>
    StorageUsage.parse({
        usedBytes: o.used ?? 0,
        userBytes: o.user ?? o.used ?? 0,
        limitBytes: o.disk?.totalBytes ?? null,
        freeBytes: o.disk?.availableBytes ?? null,
        unlimited: !o.disk && !o.quota,
        limitSource: "disk",
        quotaBytes: o.quota ?? null,
        disk: o.disk ?? null,
    });

describe("storageCardView", () => {
    it("shows the whole disk and the account's photos separately", () => {
        const v = storageCardView(
            usage({
                used: Math.round(7.4 * gib),
                disk: disk(500 * gib, 300 * gib, 200 * gib),
            }),
        );
        expect(v.headline).toBe("300 GB of 500 GB used");
        expect(v.detail).toBe("Your photos: 7.4 GB");
        expect(v.diskFraction).toBeCloseTo(0.6);
        expect(v.userFraction).toBeCloseTo(7.4 / 500, 3);
        expect(v.warning).toBeUndefined();
    });

    it.each([
        [93, "Server disk is 93% full"],
        [90, "Server disk is 90% full"],
        [89, undefined],
    ])("warns from 90%% full (%i%%)", (used, warning) => {
        const v = storageCardView(
            usage({ disk: disk(100 * gib, used * gib, (100 - used) * gib) }),
        );
        expect(v.warning).toBe(warning);
    });

    it("shows an admin quota on the second line", () => {
        const v = storageCardView(
            usage({
                used: 2 * gib,
                quota: 50 * gib,
                disk: disk(500 * gib, 100 * gib, 400 * gib),
            }),
        );
        expect(v.headline).toBe("100 GB of 500 GB used");
        expect(v.detail).toBe("Your photos: 2 GB of 50 GB");
    });

    it("without a disk (S3, no limit) shows only the account's photos", () => {
        const v = storageCardView(usage({ used: 3 * gib }));
        expect(v.headline).toBe("3 GB used");
        expect(v.detail).toBe("Your photos: 3 GB");
        expect(v.warning).toBeUndefined();
    });

    it("parses an older server without disk or userBytes", () => {
        const u = StorageUsage.parse({
            usedBytes: 5,
            limitBytes: 10,
            freeBytes: 5,
            unlimited: false,
            limitSource: "disk",
            quotaBytes: null,
        });
        expect(storageCardView(u).detail).toBe("Your photos: 5 bytes");
    });
});

describe("formatStorageBytes", () => {
    it.each([
        [0, "0 bytes"],
        [512 * 1024 * 1024, "512 MB"],
        [Math.round(7.4 * gib), "7.4 GB"],
        [69 * gib, "69 GB"],
        [500 * gib, "500 GB"],
        [Math.round(1.8 * 1024 * gib), "1.8 TB"],
    ])("%i -> %s", (bytes, text) => {
        expect(formatStorageBytes(bytes)).toBe(text);
    });
});
