import { useMediaQuery, useTheme } from "@mui/material";
import { copyTextToClipboard } from "ente-base/photovault/clipboard"; // PHOTOVAULT: clipboard
import { useCallback, useEffect, useEffectEvent, useState } from "react";

export const useIsSmallWidth = () =>
    useMediaQuery(useTheme().breakpoints.down("sm"));

// This media query is only a heuristic: a tablet with a mouse can be
// misclassified. Use it only when either result is safe.
export const useIsTouchscreen = () =>
    useMediaQuery("(hover: none) and (pointer: coarse)", { noSsr: true });

export const useClipboardCopy = (
    text: string,
): [copied: boolean, onCopy: () => void] => {
    const [copied, setCopied] = useState(false);

    const handleCopyLink = useCallback(() => {
        // PHOTOVAULT: clipboard - also works on plain http:// servers.
        void copyTextToClipboard(text).then((ok) => {
            if (!ok) return;
            setCopied(true);
            setTimeout(() => setCopied(false), 1000);
        });
    }, [text]);

    return [copied, handleCopyLink];
};

export const useInterval = (
    callback: () => void,
    delay: number | null,
): void => {
    const onTick = useEffectEvent(callback);

    useEffect(() => {
        if (delay === null) return;

        const id = setInterval(onTick, delay);
        return () => clearInterval(id);
    }, [delay]);
};
