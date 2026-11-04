import type { PublicAlbumsCredentials } from "ente-base/http";
import type { Collection } from "ente-media/collection";
import type { RefObject } from "react";

export interface JoinPublicAlbumRedirectProps {
    publicCollection?: Collection;
    accessToken?: string;
    collectionKey?: string;
    credentials?: RefObject<PublicAlbumsCredentials | undefined>;
}

const buildWebRedirectURL = (
    accessToken: string,
    collectionId: number,
    currentHash: string,
    jwtToken?: string,
): string => {
    const hashSuffix = jwtToken ? `&jwt=${encodeURIComponent(jwtToken)}` : "";
    // PHOTOVAULT: connect - Photos is served at the root of this origin
    // (outside the /albums base path), not at photos.ente.com.
    return `${window.location.origin}/?joinAlbum=${accessToken}&collectionId=${collectionId}#${currentHash}${hashSuffix}`;
};

const handleWebFallback = (
    accessToken: string,
    collectionId: number,
    currentHash: string,
    jwtToken?: string,
): void => {
    const redirectURL = buildWebRedirectURL(
        accessToken,
        collectionId,
        currentHash,
        jwtToken,
    );
    window.location.href = redirectURL;
};

export const joinPublicAlbumViaRedirect = ({
    publicCollection,
    accessToken,
    collectionKey,
    credentials,
}: JoinPublicAlbumRedirectProps) => {
    if (!publicCollection || !accessToken || !collectionKey) {
        return;
    }

    const currentHash = window.location.hash.slice(1);
    const jwtToken = credentials?.current?.accessTokenJWT;
    const collectionId = publicCollection.id;

    // PHOTOVAULT: strip - no ente:// deep link (ente://albums.ente.io on
    // Android): Ente's own app also handles it and would send the token to
    // Ente's servers. Join on the web, in Photos on this server.
    handleWebFallback(accessToken, collectionId, currentHash, jwtToken);
};
