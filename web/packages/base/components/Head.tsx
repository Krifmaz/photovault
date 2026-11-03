import Head from "next/head";
import React from "react";

interface CustomHeadProps {
    title: string;
    description?: string;
    viewportContent?: string;
}

const AlbumsFontPreloads: React.FC = () => (
    <>
        <link
            rel="preload"
            href="/fonts/inter-latin-wght-normal.woff2"
            as="font"
            type="font/woff2"
            crossOrigin="anonymous"
        />
    </>
);

export const CustomHead: React.FC<React.PropsWithChildren<CustomHeadProps>> = ({
    title,
    children,
    // PHOTOVAULT: branding
    description = "Photovault - end-to-end encrypted photos, based on Ente",
    viewportContent = "width=device-width, initial-scale=1",
}) => (
    <Head>
        {children}
        <title>{title}</title>
        <link rel="icon" href="/images/favicon.png" type="image/png" />
        <meta name="description" content={description} />
        <meta name="viewport" content={viewportContent} />
        <meta name="referrer" content="strict-origin-when-cross-origin" />
    </Head>
);

// PHOTOVAULT: strip - no Ente-hosted preview (og:image) metadata.
export const CustomHeadPhotos: React.FC<CustomHeadProps> = ({ title }) => (
    <CustomHead {...{ title }} />
);

// PHOTOVAULT: strip - no Ente-hosted preview (og:image/og:video) metadata.
export const CustomHeadAlbums: React.FC<CustomHeadProps> = ({ title }) => (
    <CustomHead {...{ title }}>
        <AlbumsFontPreloads />
    </CustomHead>
);

// PHOTOVAULT: strip - no Ente Locker title or Ente-hosted preview image.
export const CustomHeadShare: React.FC<CustomHeadProps> = ({ title }) => (
    <CustomHead {...{ title }} />
);
