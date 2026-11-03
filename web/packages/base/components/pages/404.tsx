import { useRouter } from "next/router";
import React, { useEffect } from "react";

const Page: React.FC = () => {
    // next-electron-server serves 404.html when a refreshed URL has a query.
    // Return to the SPA root instead of leaving desktop stranded here.
    const router = useRouter();

    useEffect(() => {
        void router.push("/");
    }, [router]);

    return <></>;
};

export default Page;
