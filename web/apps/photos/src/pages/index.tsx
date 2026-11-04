import { PhotosAuthShell } from "@/components/PhotosAuthShell";
import { ServerField } from "@/components/photovault/ServerField"; // PHOTOVAULT: connect
import { LoginContents } from "ente-accounts/components/LoginContents";
import { SignUpContents } from "ente-accounts/components/SignUpContents";
import { savedPartialLocalUser } from "ente-accounts/services/accounts-db";
import { CenteredFill } from "ente-base/components/containers";
import { ActivityIndicator } from "ente-base/components/mui/ActivityIndicator";
import { useBaseContext } from "ente-base/context";
import {
    JOIN_ALBUM_CONTEXT_KEY,
    type JoinAlbumContext,
} from "ente-base/join-album";
import log from "ente-base/log";
import type { ServerCheck } from "ente-base/photovault/server"; // PHOTOVAULT: connect
import { haveMasterKeyInSession } from "ente-base/session-storage";
import { savedAuthToken } from "ente-base/token";
import { canAccessIndexedDB } from "ente-gallery/services/files-db";
import { updateSessionFromElectronSafeStorageIfNeeded } from "ente-new/photos/services/account-keys";
import { t } from "i18next";
import { useRouter } from "next/router";
import React, { useCallback, useEffect, useState } from "react";

const Page: React.FC = () => {
    const { showMiniDialog } = useBaseContext();

    const [loading, setLoading] = useState(true);
    const [showLogin, setShowLogin] = useState(true);
    // PHOTOVAULT: connect - the server is shown (and changed) by ServerField
    // below the form, so the forms' own host line stays empty.
    const host = undefined;
    // Remount the forms when the server changes (Ente did this by reloading
    // the host after its developer settings dialog closed).
    const [serverKey, setServerKey] = useState(0);
    const refreshHost = useCallback(() => setServerKey((k) => k + 1), []);
    // PHOTOVAULT: connect - hide "Sign up" when the server says
    // `signupsAllowed: false` (contracts/server-info.md). A stock Ente server
    // (no /photovault/info) keeps Ente's behaviour.
    const [signupsAllowed, setSignupsAllowed] = useState(true);
    const handleServerCheck = useCallback((check: ServerCheck) => {
        const allowed = check.kind != "photovault" || check.info.signupsAllowed;
        setSignupsAllowed(allowed);
        if (!allowed) setShowLogin(true);
    }, []);

    const router = useRouter();

    const handleShowLogin = useCallback(() => setShowLogin(true), []);
    const handleShowSignUp = useCallback(() => setShowLogin(false), []);

    useEffect(() => {
        void (async () => {
            const currentURL = new URL(window.location.href);

            // Persist join data before authentication redirects discard this URL.
            const joinAlbumParam = currentURL.searchParams.get("joinAlbum");
            if (joinAlbumParam && joinAlbumParam !== "true") {
                try {
                    const accessToken = joinAlbumParam;
                    const collectionIdParam =
                        currentURL.searchParams.get("collectionId");
                    const [collectionKeyHash, fragmentParams = ""] =
                        currentURL.hash.slice(1).split("&", 2);
                    const jwtFromURL =
                        new URLSearchParams(fragmentParams).get("jwt") ??
                        undefined;

                    if (accessToken && collectionKeyHash && collectionIdParam) {
                        const { extractCollectionKeyFromShareURL } =
                            await import("ente-gallery/services/share");

                        const tempURL = new URL(window.location.href);
                        tempURL.hash = collectionKeyHash;
                        const collectionKey =
                            await extractCollectionKeyFromShareURL(tempURL);

                        const context: JoinAlbumContext = {
                            accessToken,
                            collectionKey,
                            collectionKeyHash,
                            collectionID: parseInt(collectionIdParam, 10),
                            ...(jwtFromURL && { accessTokenJWT: jwtFromURL }),
                        };

                        sessionStorage.setItem(
                            JOIN_ALBUM_CONTEXT_KEY,
                            JSON.stringify(context),
                        );
                    }
                } catch (error) {
                    log.error(
                        "Failed to store join album context from URL",
                        error,
                    );
                    showMiniDialog({
                        title: t("error"),
                        message: t("generic_error_retry"),
                    });
                }
            }

            await updateSessionFromElectronSafeStorageIfNeeded();
            if (haveMasterKeyInSession() && (await savedAuthToken())) {
                await router.push("/gallery");
            } else if (savedPartialLocalUser()?.email) {
                await router.push("/verify");
            }
            if (!(await canAccessIndexedDB())) {
                showMiniDialog({
                    title: t("error"),
                    message: t("local_storage_not_accessible"),
                    nonClosable: true,
                    cancel: false,
                });
            }
            setLoading(false);
        })();
    }, [showMiniDialog, router]);

    return (
        <TappableContainer>
            {loading ? (
                <ActivityIndicator />
            ) : (
                <PhotosAuthShell>
                    {showLogin ? (
                        <LoginContents
                            key={serverKey}
                            {...{ host }}
                            // PHOTOVAULT: connect
                            onSignUp={
                                signupsAllowed ? handleShowSignUp : undefined
                            }
                        />
                    ) : (
                        <SignUpContents
                            key={serverKey}
                            {...{ router, host }}
                            onLogin={handleShowLogin}
                        />
                    )}
                    {/* PHOTOVAULT: connect */}
                    <ServerField
                        onChange={refreshHost}
                        onCheck={handleServerCheck}
                    />
                </PhotosAuthShell>
            )}
        </TappableContainer>
    );
};

export default Page;

// PHOTOVAULT: connect - Ente's hidden "tap seven times" developer endpoint
// dialog is removed; the visible ServerField replaces it. The container keeps
// its upstream name and styling.
const TappableContainer: React.FC<React.PropsWithChildren> = ({ children }) => (
    <CenteredFill
        sx={[
            {
                bgcolor: "background.paper2",
                "@media (width <= 1024px)": { flexDirection: "column" },
            },
            (theme) =>
                theme.applyStyles("dark", { bgcolor: "background.default" }),
        ]}
    >
        {children}
    </CenteredFill>
);
