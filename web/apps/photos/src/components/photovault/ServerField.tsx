// PHOTOVAULT: connect - the server field on the login and sign-up pages. It
// replaces Ente's hidden "tap seven times" developer endpoint dialog.
//
// Collapsed it shows the server in use; expanded it takes an address, checks
// it with GET /photovault/info (Ente's /ping as a fallback) and saves it on
// this device (localStorage, kept across logout).
import { styled } from "@mui/material";
import { Button } from "ente-accounts/components/auth/Button";
import { Message } from "ente-accounts/components/auth/Message";
import {
    authBodyTypography,
    authMiniTypography,
} from "ente-accounts/components/auth/styles";
import { TextField } from "ente-accounts/components/auth/TextField";
import { TextLink } from "ente-accounts/components/auth/TextLink";
import { buildEnvCustomAPIEndpoint } from "ente-base/env";
import { customAPIOrigin, saveServerOrigin } from "ente-base/origins";
import {
    checkServer,
    normalizeServerURL,
    pageOrigin,
    resolveServerOrigin,
    savedServerStorageKey,
    serverDisplayName,
    type ServerCheck,
} from "ente-base/photovault/server";
import { t } from "i18next";
import React, { useCallback, useEffect, useState } from "react";
import { serverCheckErrorText, serverCheckSuccessText } from "./server-check";

interface ServerFieldProps {
    /** Called after the server in use has changed. */
    onChange: () => void;
    /**
     * Called with each check of the server in use (on mount and after a
     * change), e.g. to read `signupsAllowed`.
     */
    onCheck?: (check: ServerCheck) => void;
}

const savedServer = () => {
    try {
        return localStorage.getItem(savedServerStorageKey) ?? undefined;
    } catch {
        return undefined;
    }
};

/** The server used when nothing was chosen on this device. */
const defaultServer = () =>
    resolveServerOrigin({
        buildEnv: buildEnvCustomAPIEndpoint,
        page: pageOrigin(),
    });

export const ServerField: React.FC<ServerFieldProps> = ({
    onChange,
    onCheck,
}) => {
    const [current, setCurrent] = useState<string | undefined>();
    const [currentCheck, setCurrentCheck] = useState<ServerCheck | undefined>();
    const [expanded, setExpanded] = useState(false);
    const [value, setValue] = useState("");
    const [checking, setChecking] = useState<string | undefined>();
    const [error, setError] = useState<string | undefined>();
    const [success, setSuccess] = useState<string | undefined>();
    const [hasSaved, setHasSaved] = useState(false);

    const refresh = useCallback(async () => {
        const origin = await customAPIOrigin();
        setCurrent(origin);
        setHasSaved(!!savedServer());
        setCurrentCheck(undefined);
        const check = await checkServer(origin);
        setCurrentCheck(check);
        onCheck?.(check);
    }, [onCheck]);

    useEffect(() => {
        void refresh();
    }, [refresh]);

    const expand = () => {
        setValue(current ? serverDisplayName(current) : "");
        setError(undefined);
        setSuccess(undefined);
        setExpanded(true);
    };

    const connect = async (input: string) => {
        setError(undefined);
        setSuccess(undefined);
        const origin = normalizeServerURL(input);
        if (!origin) {
            setError(t("pv_server_invalid_address"));
            return;
        }
        setChecking(serverDisplayName(origin));
        const check = await checkServer(origin);
        setChecking(undefined);
        const ok = serverCheckSuccessText(check);
        if (!ok) {
            setError(serverCheckErrorText(check));
            return;
        }
        // Saving the default itself is the same as forgetting the choice, so a
        // later build-time / same-origin change still applies.
        await saveServerOrigin(origin == defaultServer() ? undefined : origin);
        setValue(serverDisplayName(origin));
        setSuccess(ok);
        setCurrent(origin);
        setCurrentCheck(check);
        setHasSaved(!!savedServer());
        onCheck?.(check);
        onChange();
    };

    const handleSubmit: React.SubmitEventHandler<HTMLFormElement> = (e) => {
        e.preventDefault();
        void connect(value);
    };

    const useDefault = () => void connect(defaultServer());

    const display = current ? serverDisplayName(current) : "";
    const currentName =
        currentCheck?.kind == "photovault" ? currentCheck.info.name : undefined;
    const currentError = currentCheck
        ? serverCheckErrorText(currentCheck)
        : undefined;

    if (!expanded) {
        return (
            <Root>
                <SummaryRow>
                    <span>{t("pv_server")}:</span>
                    <Address title={current}>
                        {currentName ? `${currentName} · ${display}` : display}
                    </Address>
                    <TextLink regular onClick={expand}>
                        {t("pv_server_change")}
                    </TextLink>
                </SummaryRow>
                {currentError && <Message kind="error">{currentError}</Message>}
            </Root>
        );
    }

    return (
        <FormRoot onSubmit={handleSubmit} noValidate>
            <TextField
                name="photovaultServer"
                label={t("pv_server_label")}
                value={value}
                onChange={(e) => {
                    setValue(e.target.value);
                    setError(undefined);
                    setSuccess(undefined);
                }}
                placeholder={t("pv_server_placeholder")}
                inputMode="url"
                autoComplete="url"
                autoFocus
                disabled={!!checking}
                error={!!error}
                helperText={error}
            />
            {checking && (
                <Message kind="info">
                    {t("pv_server_checking", { host: checking })}
                </Message>
            )}
            {success && <Message kind="success">{success}</Message>}
            <ButtonRow>
                <Button
                    type="submit"
                    variant="secondary"
                    fullWidth
                    loading={!!checking}
                >
                    {t("pv_server_connect")}
                </Button>
                <Button
                    variant="secondary"
                    fullWidth
                    disabled={!!checking}
                    onClick={() => setExpanded(false)}
                >
                    {success ? t("pv_server_done") : t("cancel")}
                </Button>
            </ButtonRow>
            {hasSaved && (
                <TextLink regular onClick={useDefault} disabled={!!checking}>
                    {t("pv_server_use_default", {
                        host: serverDisplayName(defaultServer()),
                    })}
                </TextLink>
            )}
        </FormRoot>
    );
};

const rootStyle = {
    display: "flex",
    flexDirection: "column",
    alignItems: "stretch",
    gap: "10px",
    marginTop: "20px",
    paddingTop: "16px",
    borderTop: "1px solid var(--auth-ui-stroke)",
    textAlign: "center",
} as const;

const Root = styled("div")({ ...rootStyle, alignItems: "center" });
const FormRoot = styled("form")(rootStyle);

const SummaryRow = styled("div")({
    ...authMiniTypography,
    display: "flex",
    flexWrap: "wrap",
    alignItems: "center",
    justifyContent: "center",
    gap: "6px",
    color: "var(--auth-ui-text-faint)",
    "& button": { ...authMiniTypography, fontWeight: 600 },
});

const Address = styled("span")({
    ...authMiniTypography,
    color: "var(--auth-ui-text-muted)",
    overflowWrap: "anywhere",
});

const ButtonRow = styled("div")({
    ...authBodyTypography,
    display: "flex",
    alignItems: "center",
    gap: "8px",
    // Keep the two buttons level: the auth button's 2 px hover lift stays
    // on after clicking Connect (the pointer is still over it), which left
    // Connect sitting above Cancel.
    "& > button:hover:not(:disabled)": { transform: "none" },
});
