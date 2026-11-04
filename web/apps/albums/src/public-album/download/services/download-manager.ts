import {
    getPublicAlbumsCredentials,
    requirePublicAlbumsCredentials,
    setPublicAlbumsCredentials,
} from "@/public-album/data/auth/public-link-credentials";
import {
    playableVideoURL,
    renderableImageBlob,
} from "@/public-album/media/processing/convert";
import { fetchPublicCollectionFile } from "ente-base/file-download";
import {
    retryEnsuringHTTPOk,
    type PublicAlbumsCredentials,
} from "ente-base/http";
import {
    createDownloadManager,
    type FileDownloadOpts,
    type RenderableSourceURLs,
} from "ente-gallery/services/download-core";
import type { EnteFile } from "ente-media/file";

class DownloadManager {
    private core = createDownloadManager({
        downloadThumbnail: (file) => this.downloadThumbnail(file),
        downloadFile: (file) => this.downloadFile(file),
        renderableImageBlob,
        playableVideoURL,
    });

    get publicAlbumsCredentials() {
        return getPublicAlbumsCredentials();
    }

    logout() {
        setPublicAlbumsCredentials(undefined);
        this.core.logout();
    }

    setPublicAlbumsCredentials(
        credentials: PublicAlbumsCredentials | undefined,
    ) {
        setPublicAlbumsCredentials(credentials);
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
        return publicAlbums_downloadThumbnail(
            file,
            requirePublicAlbumsCredentials(this.publicAlbumsCredentials),
        );
    }

    private async downloadFile(file: EnteFile) {
        return publicAlbums_downloadFile(
            file,
            requirePublicAlbumsCredentials(this.publicAlbumsCredentials),
        );
    }
}

export const downloadManager = new DownloadManager();

// PHOTOVAULT: strip - always fetch from the configured server (Ente's
// self-hosted path), never Ente's public-albums.ente.com CDN.
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
