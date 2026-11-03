import { CircularProgress, Typography } from "@mui/material";
import { Stack100vhCenter } from "ente-base/components/containers";
import Head from "next/head";
import type { PropsWithChildren } from "react";
// PHOTOVAULT: theme - Encore colours instead of Ente green.
import { pvGold } from "ente-base/components/utils/photovault/encore-tokens";

function PublicMemoryDocumentHead() {
    return (
        <Head>
            <meta name="robots" content="noindex, nofollow" />
        </Head>
    );
}

export function PublicMemoryPageShell({ children }: PropsWithChildren) {
    return (
        <>
            <PublicMemoryDocumentHead />
            {children}
        </>
    );
}

export function PublicMemoryLoadingContent() {
    return (
        <Stack100vhCenter sx={{ minHeight: "100dvh" }}>
            {/* PHOTOVAULT: theme */}
            <CircularProgress sx={{ color: pvGold }} size={32} />
        </Stack100vhCenter>
    );
}

export function PublicMemoryLoadingState() {
    return (
        <PublicMemoryPageShell>
            <PublicMemoryLoadingContent />
        </PublicMemoryPageShell>
    );
}

export function PublicMemoryErrorState({ message }: { message: string }) {
    return (
        <PublicMemoryPageShell>
            <Stack100vhCenter>
                <Typography sx={{ color: "critical.main" }}>
                    {message}
                </Typography>
            </Stack100vhCenter>
        </PublicMemoryPageShell>
    );
}

export function PublicMemoryEmptyState() {
    return (
        <PublicMemoryPageShell>
            <Stack100vhCenter>
                <Typography>No photos found in this memory.</Typography>
            </Stack100vhCenter>
        </PublicMemoryPageShell>
    );
}
