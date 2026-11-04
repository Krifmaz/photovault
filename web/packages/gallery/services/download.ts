import {
    fetchFile,
    fetchPublicCollectionFile,
    fetchPublicMemoryFile,
} from "ente-base/file-download";
import {
    retryEnsuringHTTPOk,
    type PublicAlbumsCredentials,
} from "ente-base/http";
import type { PublicMemoryCredentials } from "ente-base/public-memory";
import type { EnteFile } from "ente-media/file";
import { playableVideoURL, renderableImageBlob } from "./convert";
import {
    createDownloadManager,
    isNetworkDownloadError,
    type FileDownloadOpts,
    type RenderableSourceURLs,
} from "./download-core";
import { hlsPlaylistDataForFile, type HLSPlaylistDataForFile } from "./video";

export { isNetworkDownloadError };
export type { RenderableSourceURLs };

class DownloadManager {
    publicAlbumsCredentials: PublicAlbumsCredentials | undefined;
    publicMemoryCredentials: PublicMemoryCredentials | undefined;

    private core = createDownloadManager({
        downloadThumbnail: (file) => this.downloadThumbnail(file),
        downloadFile: (file) => this.downloadFile(file),
        renderableImageBlob,
        playableVideoURL,
    });

    logout() {
        this.publicAlbumsCredentials = undefined;
        this.publicMemoryCredentials = undefined;
        this.core.logout();
    }

    setPublicAlbumsCredentials(
        credentials: PublicAlbumsCredentials | undefined,
    ) {
        this.publicAlbumsCredentials = credentials;
    }

    setPublicMemoryCredentials(
        credentials: PublicMemoryCredentials | undefined,
    ) {
        this.publicMemoryCredentials = credentials;
    }

    fileDownloadProgressSubscribe(onChange: () => void) {
        return this.core.fileDownloadProgressSubscribe(onChange);
    }

    fileDownloadProgressSnapshot() {
        return this.core.fileDownloadProgressSnapshot();
    }

    renderableThumbnailURL(file: EnteFile, cachedOnly = false) {
        return this.core.renderableThumbnailURL(file, cachedOnly);
    }

    thumbnailData(file: EnteFile, cachedOnly = false) {
        return this.core.thumbnailData(file, cachedOnly);
    }

    hlsPlaylistDataForPublicMemory = async (
        file: EnteFile,
    ): Promise<HLSPlaylistDataForFile> => {
        if (!this.publicMemoryCredentials) return undefined;
        return hlsPlaylistDataForFile(
            file,
            undefined,
            this.publicMemoryCredentials,
        );
    };

    renderableSourceURLs(file: EnteFile): Promise<RenderableSourceURLs> {
        return this.core.renderableSourceURLs(file);
    }

    fileBlob(file: EnteFile, opts?: FileDownloadOpts) {
        return this.core.fileBlob(file, opts);
    }

    fileStream(file: EnteFile, opts?: FileDownloadOpts) {
        return this.core.fileStream(file, opts);
    }

    private async downloadThumbnail(file: EnteFile) {
        if (this.publicMemoryCredentials) {
            return publicMemory_downloadThumbnail(
                file,
                this.publicMemoryCredentials,
            );
        } else if (this.publicAlbumsCredentials) {
            return publicAlbums_downloadThumbnail(
                file,
                this.publicAlbumsCredentials,
            );
        } else {
            return photos_downloadThumbnail(file);
        }
    }

    // PHOTOVAULT: strip - `opts.background` only chose between the server
    // and Ente's files.ente.com proxy, so it is not needed here.
    private async downloadFile(file: EnteFile) {
        if (this.publicAlbumsCredentials) {
            return publicAlbums_downloadFile(
                file,
                this.publicAlbumsCredentials,
            );
        } else if (this.publicMemoryCredentials) {
            return publicMemory_downloadFile(
                file,
                this.publicMemoryCredentials,
            );
        } else {
            return photos_downloadFile(file);
        }
    }
}

export const downloadManager = new DownloadManager();

// PHOTOVAULT: strip - thumbnails and files always come from the configured
// server (Ente's self-hosted path). Ente's own builds fetched them from its
// thumbnails/files/public-albums CDN hosts when no custom origin was set.
const photos_downloadThumbnail = async (file: EnteFile) => {
    const res = await retryEnsuringHTTPOk(() =>
        fetchFile(file.id, "thumbnail"),
    );
    return new Uint8Array(await res.arrayBuffer());
};

const photos_downloadFile = (file: EnteFile) =>
    retryEnsuringHTTPOk(() => fetchFile(file.id, "file"));

const publicAlbums_downloadThumbnail = async (
    file: EnteFile,
    credentials: PublicAlbumsCredentials,
) => {
    const res = await retryEnsuringHTTPOk(() =>
        fetchPublicCollectionFile(file.id, "thumbnail", credentials),
    );
    return new Uint8Array(await res.arrayBuffer());
};

const publicAlbums_downloadFile = (
    file: EnteFile,
    credentials: PublicAlbumsCredentials,
) =>
    retryEnsuringHTTPOk(() =>
        fetchPublicCollectionFile(file.id, "file", credentials),
    );

const publicMemory_downloadThumbnail = async (
    file: EnteFile,
    credentials: PublicMemoryCredentials,
) => {
    const getThumbnail = () =>
        fetchPublicMemoryFile(file.id, "thumbnail", credentials);

    const res = await retryEnsuringHTTPOk(getThumbnail);
    return new Uint8Array(await res.arrayBuffer());
};

const publicMemory_downloadFile = async (
    file: EnteFile,
    credentials: PublicMemoryCredentials,
) => {
    const getFile = () => fetchPublicMemoryFile(file.id, "file", credentials);

    return retryEnsuringHTTPOk(getFile);
};
