import AutoAwesomeOutlinedIcon from "@mui/icons-material/AutoAwesomeOutlined";
import type { MiniDialogAttributes } from "ente-base/components/MiniDialog";
import { ensureElectron } from "ente-base/electron";
import type { AppUpdate } from "ente-base/types/ipc";
import { t } from "i18next";

// PHOTOVAULT: strip - no link to Ente's download page; the dialog only says
// that the operation needs the desktop app.
export const downloadAppDialogAttributes = (): MiniDialogAttributes => ({
    title: t("download_app"),
    message: t("download_app_message"),
    cancel: t("ok"),
});

export const updateReadyToInstallDialogAttributes = ({
    version,
}: AppUpdate): MiniDialogAttributes => ({
    title: t("update_available"),
    message: t("update_installable_message"),
    icon: <AutoAwesomeOutlinedIcon />,
    nonClosable: true,
    continue: {
        text: t("install_now"),
        action: () => ensureElectron().updateAndRestart(),
    },
    cancel: {
        text: t("install_on_next_launch"),
        action: () => ensureElectron().updateOnNextRestart(version),
    },
});

export const updateAvailableForDownloadDialogAttributes = ({
    version,
}: AppUpdate): MiniDialogAttributes => ({
    title: t("update_available"),
    message: t("update_available_message"),
    icon: <AutoAwesomeOutlinedIcon />,
    // PHOTOVAULT: strip - no "Download and install" link to Ente's site.
    cancel: {
        text: t("ignore_this_version"),
        action: () => ensureElectron().skipAppUpdate(version),
    },
});
