import "package:dio/dio.dart";
import "package:photos/service_locator.dart";

const _v3RetryDelay = Duration(hours: 1);
DateTime? _v3RetryAfter;

enum FileUrlType {
  download,
  publicDownload,
  thumbnail,
  publicThumbnail,
  directDownload,
}

class FileUrl {
  static Future<String?> tryGetV3Url(
    Dio dio,
    int fileID,
    FileUrlType type, {
    required Map<String, dynamic> headers,
    CancelToken? cancelToken,
  }) async {
    // PHOTOVAULT: strip - upstream skipped v3 URLs when Ente's own CDN
    // workers were used (endpoint == Ente's api host); a configured server never
    // uses them.
    if (_v3RetryAfter?.isAfter(DateTime.now()) ?? false) {
      return null;
    }

    final response = await dio.get<Object?>(
      _v3Path(fileID, type),
      options: Options(
        headers: headers,
        validateStatus: (status) => status == 200 || status == 404,
      ),
      cancelToken: cancelToken,
    );
    if (response.statusCode == 404) {
      // TODO(migration): Remove the legacy fallback once all supported Museum
      // versions provide the v3 file URL endpoints. Started 4 Aug 2026.
      _v3RetryAfter = DateTime.now().add(_v3RetryDelay);
      return null;
    }
    return (response.data as Map<String, dynamic>)["url"] as String;
  }

  // PHOTOVAULT: strip - always the configured server's own paths (what
  // upstream uses for self-hosted servers); the files/thumbnails/
  // public-albums worker URLs on Ente's hosts are removed.
  static String getLegacyUrl(int fileID, FileUrlType type) {
    final endpoint = endpointConfig.endpoint;

    switch (type) {
      case FileUrlType.directDownload:
      case FileUrlType.download:
        return "$endpoint/files/download/$fileID";

      case FileUrlType.publicDownload:
        return "$endpoint/public-collection/files/download/$fileID";

      case FileUrlType.thumbnail:
        return "$endpoint/files/preview/$fileID";

      case FileUrlType.publicThumbnail:
        return "$endpoint/public-collection/files/preview/$fileID";
    }
  }

  static String _v3Path(int fileID, FileUrlType type) {
    switch (type) {
      case FileUrlType.download:
      case FileUrlType.directDownload:
        return "/files/download/v3/$fileID";
      case FileUrlType.publicDownload:
        return "/public-collection/files/download/v3/$fileID";
      case FileUrlType.thumbnail:
        return "/files/thumbnail/v3/$fileID";
      case FileUrlType.publicThumbnail:
        return "/public-collection/files/thumbnail/v3/$fileID";
    }
  }
}
