// PHOTOVAULT: connect - Settings → Server: what /photovault/info says about
// the server this app talks to, and a QR code that connects the Photovault
// phone app to it (payload from contracts/server-info.md).
import { Box, Stack, Typography } from "@mui/material";
import { ActivityIndicator } from "ente-base/components/mui/ActivityIndicator";
import { FocusVisibleButton } from "ente-base/components/mui/FocusVisibleButton";
import {
    TitledNestedSidebarDrawer,
    type NestedSidebarDrawerVisibilityProps,
} from "ente-base/components/mui/SidebarDrawer";
import { customAPIOrigin } from "ente-base/origins";
import {
    checkServer,
    connectQRPayload,
    type ServerCheck,
} from "ente-base/photovault/server";
import { t } from "i18next";
import encodeQR from "qr";
import React, { useCallback, useEffect, useMemo, useState } from "react";
import {
    serverCheckErrorText,
    serverCheckSuccessText,
} from "../../photovault/server-check";

export const ServerSettings: React.FC<NestedSidebarDrawerVisibilityProps> = ({
    open,
    onClose,
    onRootClose,
}) => {
    const [check, setCheck] = useState<ServerCheck | undefined>();

    const load = useCallback(async () => {
        setCheck(undefined);
        setCheck(await checkServer(await customAPIOrigin()));
    }, []);

    useEffect(() => {
        if (open) void load();
    }, [open, load]);

    const handleRootClose = () => {
        onClose();
        onRootClose();
    };

    return (
        <TitledNestedSidebarDrawer
            {...{ open, onClose }}
            onRootClose={handleRootClose}
            title={t("pv_server")}
        >
            <Stack sx={{ px: 2, py: 1, gap: 2 }}>
                {!check ? (
                    <Box
                        sx={{
                            py: 4,
                            display: "flex",
                            justifyContent: "center",
                        }}
                    >
                        <ActivityIndicator />
                    </Box>
                ) : (
                    <ServerDetails check={check} onRetry={load} />
                )}
            </Stack>
        </TitledNestedSidebarDrawer>
    );
};

const ServerDetails: React.FC<{ check: ServerCheck; onRetry: () => void }> = ({
    check,
    onRetry,
}) => {
    const error = serverCheckErrorText(check);
    if (error) {
        return (
            <Stack sx={{ gap: 2, px: 1 }}>
                <Typography sx={{ color: "critical.main" }}>{error}</Typography>
                <FocusVisibleButton color="secondary" onClick={onRetry}>
                    {t("pv_server_retry")}
                </FocusVisibleButton>
            </Stack>
        );
    }

    const info = check.kind == "photovault" ? check.info : undefined;
    // A stock Ente server has no /photovault/info; the QR then carries the
    // bare address, which the app's scanner also accepts.
    const qrPayload = info ? connectQRPayload(info) : check.origin;
    const notSet = t("pv_server_not_set");

    return (
        <>
            <Typography variant="small" sx={{ color: "text.muted", px: 1 }}>
                {serverCheckSuccessText(check)}
            </Typography>
            <Box
                component="dl"
                sx={{
                    m: 0,
                    px: 1,
                    display: "grid",
                    gridTemplateColumns: "auto 1fr",
                    columnGap: 2,
                    rowGap: 1.5,
                    "& dt": { color: "text.muted" },
                    "& dd": { m: 0, overflowWrap: "anywhere" },
                }}
            >
                {info && (
                    <>
                        <Detail label={t("pv_server_name")} value={info.name} />
                        <Detail
                            label={t("pv_server_version")}
                            value={info.version}
                        />
                    </>
                )}
                <Detail
                    label={t("pv_server_home_address")}
                    value={info ? (info.homeUrl ?? notSet) : notSet}
                />
                <Detail
                    label={t("pv_server_public_address")}
                    value={info?.publicUrl ?? check.origin}
                />
            </Box>
            <Stack
                sx={{
                    alignItems: "center",
                    gap: 1.5,
                    p: 2,
                    borderRadius: 2,
                    bgcolor: "fill.faint",
                }}
            >
                <Typography sx={{ fontWeight: 600 }}>
                    {t("pv_server_qr_title")}
                </Typography>
                <QRCode text={qrPayload} />
                <Typography
                    variant="small"
                    sx={{ color: "text.muted", textAlign: "center" }}
                >
                    {t("pv_server_qr_hint")}
                </Typography>
            </Stack>
        </>
    );
};

const Detail: React.FC<{ label: string; value: string }> = ({
    label,
    value,
}) => (
    <>
        <Typography component="dt" variant="small">
            {label}
        </Typography>
        <Typography component="dd" variant="small">
            {value}
        </Typography>
    </>
);

/**
 * The QR code as an SVG path (dark modules on white, 4-module quiet zone,
 * medium error correction), drawn from the `qr` package's raw bitmap.
 */
const QRCode: React.FC<{ text: string }> = ({ text }) => {
    const { size, d } = useMemo(() => {
        const rows = encodeQR(text, "raw", { ecc: "medium", border: 4 });
        let d = "";
        rows.forEach((row, y) =>
            row.forEach((dark, x) => {
                if (dark) d += `M${x} ${y}h1v1h-1z`;
            }),
        );
        return { size: rows.length, d };
    }, [text]);

    return (
        <Box
            component="svg"
            role="img"
            aria-label={text}
            viewBox={`0 0 ${size} ${size}`}
            shapeRendering="crispEdges"
            sx={{ width: 220, height: 220, bgcolor: "#fff", borderRadius: 1 }}
        >
            <title>{text}</title>
            <rect width={size} height={size} fill="#fff" />
            <path d={d} fill="#000" />
        </Box>
    );
};
