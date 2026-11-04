import { useAuthPageConfig } from "ente-accounts/components/auth/AuthPageProvider";
import { LoginContents } from "ente-accounts/components/LoginContents";
import { savedPartialLocalUser } from "ente-accounts/services/accounts-db";
import { LoadingIndicator } from "ente-base/components/loaders";
import { customAPIHost } from "ente-base/origins";
import type { ServerCheck } from "ente-base/photovault/server"; // PHOTOVAULT: connect
import { useRouter } from "next/router";
import React, { useCallback, useEffect, useState } from "react";

const Page: React.FC = () => {
    const { Shell, LoginFrame, keepLoginLoadingOnRedirect, ServerField } =
        useAuthPageConfig();
    // PHOTOVAULT: connect - remount the form when the server changes, and
    // hide "Sign up" when the server says `signupsAllowed: false`.
    const [serverKey, setServerKey] = useState(0);
    const handleServerChange = useCallback(
        () => setServerKey((k) => k + 1),
        [],
    );
    const [signupsAllowed, setSignupsAllowed] = useState(true);
    const handleServerCheck = useCallback(
        (check: ServerCheck) =>
            setSignupsAllowed(
                check.kind != "photovault" || check.info.signupsAllowed,
            ),
        [],
    );
    const [loading, setLoading] = useState(true);
    const [host, setHost] = useState<string | undefined>(undefined);

    const router = useRouter();

    const refreshHost = useCallback(
        () => void customAPIHost().then(setHost),
        [],
    );

    useEffect(() => {
        refreshHost();
        if (savedPartialLocalUser()?.email) {
            void router.replace("/verify");
            if (keepLoginLoadingOnRedirect) return;
        }
        setLoading(false);
    }, [router, refreshHost, keepLoginLoadingOnRedirect]);

    const onSignUp = useCallback(() => void router.push("/signup"), [router]);

    if (loading) return <LoadingIndicator />;

    const contents = (
        <Shell>
            {/* PHOTOVAULT: connect - the server field instead of the host. */}
            <LoginContents
                key={serverKey}
                host={ServerField ? undefined : host}
                onSignUp={signupsAllowed ? onSignUp : undefined}
            />
            {ServerField && (
                <ServerField
                    onChange={handleServerChange}
                    onCheck={handleServerCheck}
                />
            )}
        </Shell>
    );

    return LoginFrame ? (
        <LoginFrame onHostChanged={refreshHost}>{contents}</LoginFrame>
    ) : (
        contents
    );
};

export default Page;
