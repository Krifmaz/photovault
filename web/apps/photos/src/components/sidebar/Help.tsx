import { Stack, Typography } from "@mui/material";
import {
    TitledNestedSidebarDrawer,
    type NestedSidebarDrawerVisibilityProps,
} from "ente-base/components/mui/SidebarDrawer";
import {
    pvBrandName,
    pvCredit,
} from "ente-base/components/utils/photovault/encore-tokens"; // PHOTOVAULT: credit
import { RowCard } from "ente-base/components/v2/RowCard";
import { useBaseContext } from "ente-base/context";
import log from "ente-base/log";
import { savedLogs } from "ente-base/log-web";
import { saveStringAsFile } from "ente-base/utils/web";
import type { SidebarActionID } from "ente-new/photos/services/search/types";
import { t } from "i18next";
import React, { useCallback, useEffect, useState } from "react";
import { Trans } from "react-i18next";

export type HelpAction = Extract<
    SidebarActionID,
    | "help.helpCenter"
    | "help.blog"
    | "help.requestFeature"
    | "help.support"
    | "help.viewLogs"
>;

type HelpProps = NestedSidebarDrawerVisibilityProps & {
    pendingAction?: HelpAction;
    onActionHandled?: (Action?: HelpAction) => void;
};

export const Help: React.FC<HelpProps> = ({
    open,
    onClose,
    onRootClose,
    pendingAction,
    onActionHandled,
}) => {
    const { showMiniDialog } = useBaseContext();
    const [appVersion, setAppVersion] = useState("");

    useEffect(() => {
        void globalThis.electron?.appVersion().then(setAppVersion);
    }, []);

    const handleRootClose = () => {
        onClose();
        onRootClose();
    };

    // PHOTOVAULT: strip - Ente's help centre, blog, feature requests and
    // support e-mail are removed; only "View logs" and About remain.

    const viewLogs = useCallback(async () => {
        log.info("Viewing logs");
        const electron = globalThis.electron;
        if (electron) {
            await electron.openLogDirectory();
        } else {
            // PHOTOVAULT: strip - file name without "ente".
            saveStringAsFile(
                savedLogs(),
                `photovault-web-logs-${Date.now()}.txt`,
            );
        }
    }, []);

    const confirmViewLogs = useCallback(
        () =>
            showMiniDialog({
                title: t("view_logs"),
                message: <Trans i18nKey={"view_logs_message"} />,
                continue: { text: t("view_logs"), action: viewLogs },
            }),
        [showMiniDialog, viewLogs],
    );

    useEffect(() => {
        if (!open || !pendingAction) return;
        switch (pendingAction) {
            // PHOTOVAULT: strip - Ente help links removed.
            case "help.helpCenter":
            case "help.blog":
            case "help.requestFeature":
            case "help.support":
                break;
            case "help.viewLogs":
                confirmViewLogs();
                break;
        }
        onActionHandled?.();
    }, [confirmViewLogs, open, onActionHandled, pendingAction]);

    return (
        <TitledNestedSidebarDrawer
            maxWidth="440px"
            {...{ open, onClose }}
            onRootClose={handleRootClose}
            title={t("help")}
        >
            <Stack sx={{ px: 2, py: 1, gap: 1 }}>
                {/* PHOTOVAULT: strip - Ente help, blog, feature request and support rows removed. */}
                <RowCard
                    title={t("view_logs")}
                    subtitle="Share with support when something goes wrong"
                    onClick={confirmViewLogs}
                />
                {/* PHOTOVAULT: credit - About, with the plain-text "Based on Ente" line (no link, no logo). */}
                <Stack sx={{ px: 1, pt: 2, gap: 0.5 }} aria-label="About">
                    <Typography variant="small" sx={{ fontWeight: 600 }}>
                        {pvBrandName}
                        {appVersion ? ` ${appVersion}` : ""}
                    </Typography>
                    <Typography variant="small" sx={{ color: "text.muted" }}>
                        {pvCredit}
                    </Typography>
                </Stack>
            </Stack>
        </TitledNestedSidebarDrawer>
    );
};
