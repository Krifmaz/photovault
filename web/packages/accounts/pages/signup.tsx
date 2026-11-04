import { useAuthPageConfig } from "ente-accounts/components/auth/AuthPageProvider";
import { SignUpContents } from "ente-accounts/components/SignUpContents";
import { savedPartialLocalUser } from "ente-accounts/services/accounts-db";
import { LoadingIndicator } from "ente-base/components/loaders";
import { customAPIHost } from "ente-base/origins";
import { useRouter } from "next/router";
import React, { useCallback, useEffect, useState } from "react";

const Page: React.FC = () => {
    const { Shell, ServerField } = useAuthPageConfig();
    // PHOTOVAULT: connect - remount the form when the server changes.
    const [serverKey, setServerKey] = useState(0);
    const handleServerChange = useCallback(
        () => setServerKey((k) => k + 1),
        [],
    );
    const [loading, setLoading] = useState(true);
    const [host, setHost] = useState<string | undefined>(undefined);

    const router = useRouter();

    useEffect(() => {
        void customAPIHost().then(setHost);
        if (savedPartialLocalUser()?.email) void router.replace("/verify");
        setLoading(false);
    }, [router]);

    const onLogin = useCallback(() => void router.push("/login"), [router]);

    return loading ? (
        <LoadingIndicator />
    ) : (
        <Shell>
            {/* PHOTOVAULT: connect - the server field instead of the host. */}
            <SignUpContents
                key={serverKey}
                {...{ router, onLogin }}
                host={ServerField ? undefined : host}
            />
            {ServerField && <ServerField onChange={handleServerChange} />}
        </Shell>
    );
};

export default Page;
