import "package:flutter/foundation.dart";

const int thumbnailSmallSize = 256;
const int thumbnailQuality = 50;
const int thumbnailLargeSize = 512;
const int compressedThumbnailResolution = 1080;
const int thumbnailDataLimit = 100 * 1024;
// PHOTOVAULT: telemetry removed (Sentry DSNs and tunnel); Ente's GitHub
// discussions link removed with Help & Support.
const int microSecondsInDay = 86400000000;
const int android11SDKINT = 30;
const int android12SDKINT = 31;
const int jan011981Time = 347155200000000;
const int galleryLoadStartTime = -8000000000000000; // June 28, 1716
const int galleryLoadEndTime = 9223372036854775807; // 2^63 - 1
const int batchSize = 1000;
const int batchSizeCopy = 100;
const photoGridSizeDefault = 3;
const photoGridSizeMin = 3;
const photoGridSizeMax = 6;
const subGalleryMultiplier = 10;

const String sharedMediaIdentifier = 'ente-shared-media://';

const galleryThumbnailDiskLoadDeferDuration = Duration(milliseconds: 80);
const galleryThumbnailServerLoadDeferDuration = Duration(milliseconds: 80);
const mediaManagementHintDismissDuration = Duration(days: 7);
const mediaManagementHintDeleteAttemptThreshold = 3;

const dragSensitivity = 8;

// PHOTOVAULT: strip - no support address (Ente's was hard-coded); the
// server contract (`/photovault/info`) has no admin contact, so there is
// none. Upstream "Contact support" buttons pass this to `sendLogs`
// (utils/email_util.dart), which ignores it and lets the person view or
// export the logs for their server's administrator.
const supportEmail = '';

const multipartPartSize = 20 * 1024 * 1024;

// PHOTOVAULT: strip - removed kDefaultProductionEndpoint (Ente's api host) and
// kLegacyProductionEndpoint (Ente's old api host): the endpoint is always the
// configured server (core/network/endpoint_config.dart).
// Ente's Cloudflare upload worker (was Ente's uploader host). Only used when
// `EndpointConfig.isProduction`, which is always false, so it is never
// contacted; it points at nothing instead of an Ente host.
const kUploadProxyEndpoint = '';
// PHOTOVAULT: removed kPhotosWebDomain (photos.ente.com) and kFamilyUrl.

const int intMaxValue = 9223372036854775807;

// Cap layouts at the 430-point width of an iPhone 14 Pro Max.
const double restrictedMaxWidth = 430;

const double mobileSmallThreshold = 336;

// Note: 0 indicates no device limit
const publicLinkDeviceLimits = [0, 50, 25, 10, 5, 2, 1];

const kilometersPerDegree = 111.16;

const defaultRadiusValues = <double>[1, 2, 10, 20, 40, 80, 200, 400, 1200];

const defaultRadiusValue = 40.0;

const galleryGridSpacing = 1.0;

const kSearchSectionLimit = 9;

const maxPickAssetLimit = 50;

const iOSGroupIDMemory = "group.io.ente.frame.EnteMemoryWidget";

const blackThumbnailBase64 =
    '/9j/4AAQSkZJRgABAQAAAQABAAD/2wBDAAEBAQEBAQEB'
    'AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQH/2wBDAQEBAQEBAQ'
    'EBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQH/wAARC'
    'ACWASwDAREAAhEBAxEB/8QAHwAAAQUBAQEBAQEAAAAAAAAAAAECAwQFBgcICQoL/8QAtRAAAgEDAwIEAwUF'
    'BAQAAAF9AQIDAAQRBRIhMUEGE1FhByJxFDKBkaEII0KxwRVS0fAkM2JyggkKFhcYGRolJicoKSo0NTY3ODk'
    '6Q0RFRkdISUpTVFVWV1hZWmNkZWZnaGlqc3R1dnd4eXqDhIWGh4iJipKTlJWWl5iZmqKjpKWmp6ipqrKztL'
    'W2t7i5usLDxMXGx8jJytLT1NXW19jZ2uHi4+Tl5ufo6erx8vP09fb3+Pn6/8QAHwEAAwEBAQEBAQEBAQAAA'
    'AAAAAECAwQFBgcICQoL/8QAtREAAgECBAQDBAcFBAQAAQJ3AAECAxEEBSExBhJBUQdhcRMiMoEIFEKRobHBCSMzUvAVY'
    'nLRChYkNOEl8RcYGRomJygpKjU2Nzg5OkNERUZHSElKU1RVVldYWVpjZGVmZ2hpanN0dXZ3eHl6goOEhYaHiImK'
    'kpOUlZaXmJmaoqOkpaanqKmqsrO0tba3uLm6wsPExcbHyMnK0tPU1dbX2Nna4uPk5ebn6Onq8vP09fb3+Pn6/9oAD'
    'AMBAAIRAxEAPwD/AD/6ACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKA'
    'CgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACg'
    'AoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKAC'
    'gAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAo'
    'AKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACg'
    'AoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACg'
    'AoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKA'
    'CgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKA'
    'CgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoA'
    'KACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACg'
    'AoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAo'
    'AKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKA'
    'CgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAK'
    'ACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoA'
    'KACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAo'
    'AKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAo'
    'AKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgAoAKACgD/9k=';

const uploadTempFilePrefix = "upload_file_";
final tempDirCleanUpInterval = kDebugMode
    ? const Duration(hours: 1).inMicroseconds
    : const Duration(hours: 6).inMicroseconds;

const kMaxAppbarFilters = 14;

const kLivePhotoHashSeparator = ':';
