# API compatibility matrix

Every route registered in museum's `cmd/museum/main.go` (ente-io/ente
`b2ed3dc`, 2026-11-03), with the authentication group it sits in and its
status in Photovault.

Status values:

* **planned**: not implemented yet.
* **done**: implemented and covered by tests against the museum contract.
* **stub**: answers with a fixed museum-compatible response (cloud-only
  features such as billing).
* **n/a**: deliberately not provided (reason in the notes).

Auth groups: *public* (no token), *private* (`X-Auth-Token`), *storage*
(private, and refused for the Auth app), *admin* (private + admin user),
the JWT groups (payment, family, accounts) and the share-token groups
(public collection, file link, public memory, cast).

## museum routes

| Method | Path | Auth | museum handler | Status |
| --- | --- | --- | --- | --- |
| GET | `/ping` | public | `(inline)` | done |
| GET | `/fire/db-m-ping` | public | `(inline)` | done |
| POST | `/events` | public | `eventHandler.Create` | done |
| POST | `/events/user` | private | `eventHandler.CreateForUser` | done |
| GET | `/files/upload-eligibility` | storage | `fileHandler.ValidateUploadEligibility` | done |
| GET | `/files/upload-urls` | storage | `fileHandler.GetUploadURLs` | done |
| GET | `/files/multipart-upload-urls` | storage | `fileHandler.GetMultipartUploadURLs` | done |
| POST | `/files/upload-url` | storage | `fileHandler.GetUploadURLV2` | done |
| POST | `/files/multipart-upload-url` | storage | `fileHandler.GetMultipartUploadURLV2` | done |
| GET | `/files/download/{fileID}` | storage | `fileHandler.Get` | done |
| GET | `/files/download/v2/{fileID}` | storage | `fileHandler.GetURL` | done |
| GET | `/files/download/v3/{fileID}` | storage | `fileHandler.GetURLV3` | done |
| GET | `/files/preview/{fileID}` | storage | `fileHandler.GetThumbnail` | done |
| GET | `/files/preview/v2/{fileID}` | storage | `fileHandler.GetThumbnailURL` | done |
| GET | `/files/thumbnail/v3/{fileID}` | storage | `fileHandler.GetThumbnailURLV3` | done |
| POST | `/files/share-url` | storage | `fileHandler.ShareUrl` | done |
| GET | `/files/share-url` | storage | `fileHandler.GetUrls` | done |
| PUT | `/files/share-url` | storage | `fileHandler.UpdateFileURL` | done |
| DELETE | `/files/share-url/{fileID}` | storage | `fileHandler.DisableUrl` | done |
| GET | `/files/share-urls/` | storage | `fileHandler.GetUrls` | done |
| PUT | `/files/data` | storage | `fileHandler.PutFileData` | done |
| PUT | `/files/video-data` | storage | `fileHandler.PutVideoData` | done |
| POST | `/files/data/status-diff` | storage | `fileHandler.FileDataStatusDiff` | done |
| POST | `/files/data/fetch` | storage | `fileHandler.GetFilesData` | done |
| GET | `/files/data/fetch` | storage | `fileHandler.GetFileData` | done |
| GET | `/files/data/preview-upload-url` | storage | `fileHandler.GetPreviewUploadURL` | done |
| POST | `/files/data/preview-upload-url` | storage | `fileHandler.GetPreviewUploadURLV2` | done |
| POST | `/files/data/multipart-preview-upload-url` | storage | `fileHandler.GetMultipartPreviewUploadURL` | done |
| GET | `/files/data/preview` | storage | `fileHandler.GetPreviewURL` | done |
| POST | `/files` | storage | `fileHandler.CreateOrUpdate` | done |
| POST | `/files/meta` | storage | `fileHandler.CreateMetaFile` | done |
| POST | `/files/copy` | storage | `fileHandler.CopyFiles` | done |
| PUT | `/files/update` | storage | `fileHandler.Update` | done |
| POST | `/files/trash` | storage | `fileHandler.Trash` | done |
| POST | `/files/size` | storage | `fileHandler.GetSize` | done |
| POST | `/files/info` | storage | `fileHandler.GetInfo` | done |
| GET | `/files/duplicates` | storage | `fileHandler.GetDuplicates` | done |
| PUT | `/files/thumbnail` | storage | `fileHandler.UpdateThumbnail` | done |
| PUT | `/files/magic-metadata` | storage | `fileHandler.UpdateMagicMetadata` | done |
| PUT | `/files/public-magic-metadata` | storage | `fileHandler.UpdatePublicMagicMetadata` | done |
| GET | `/files/count` | public | `fileHandler.GetTotalFileCount` | done |
| POST | `/paste/create` | public | `pasteHandler.Create` | done |
| POST | `/paste/guard` | public | `pasteHandler.Guard` | done |
| POST | `/paste/consume` | public | `pasteHandler.Consume` | done |
| GET | `/trash/v2/diff` | storage | `trashHandler.GetDiffV2` | done |
| POST | `/trash/delete` | storage | `trashHandler.Delete` | done |
| POST | `/trash/empty` | storage | `trashHandler.Empty` | done |
| GET | `/comments/diff` | storage | `commentsHandler.Diff` | done |
| POST | `/comments` | storage | `commentsHandler.Create` | done |
| PUT | `/comments/{commentID}` | storage | `commentsHandler.Update` | done |
| DELETE | `/comments/{commentID}` | storage | `commentsHandler.Delete` | done |
| GET | `/reactions/diff` | storage | `reactionsHandler.Diff` | done |
| PUT | `/reactions` | storage | `reactionsHandler.Upsert` | done |
| DELETE | `/reactions/{reactionID}` | storage | `reactionsHandler.Delete` | done |
| GET | `/social/diff` | storage | `socialHandler.UnifiedDiff` | done |
| GET | `/social/anon-profiles` | storage | `socialHandler.AnonProfiles` | done |
| GET | `/comments-reactions/updated-at` | storage | `socialHandler.LatestUpdates` | done |
| POST | `/users/ott` | public | `userHandler.SendOTT` | done |
| POST | `/users/verify-email` | public | `userHandler.VerifyEmail` | done |
| POST | `/users/two-factor/verify` | public | `userHandler.VerifyTwoFactor` | done |
| GET | `/users/two-factor/recover` | public | `userHandler.RecoverTwoFactor` | done |
| POST | `/users/two-factor/remove` | public | `userHandler.RemoveTwoFactor` | done |
| POST | `/users/two-factor/passkeys/begin` | public | `userHandler.BeginPasskeyAuthenticationCeremony` | done |
| POST | `/users/two-factor/passkeys/finish` | public | `userHandler.FinishPasskeyAuthenticationCeremony` | done |
| GET | `/users/two-factor/passkeys/get-token` | public | `userHandler.GetTokenForPasskeySession` | done |
| GET | `/users/two-factor/recovery-status` | private | `userHandler.GetTwoFactorRecoveryStatus` | done |
| POST | `/users/two-factor/passkeys/configure-recovery` | private | `userHandler.ConfigurePasskeyRecovery` | done |
| GET | `/users/two-factor/status` | private | `userHandler.GetTwoFactorStatus` | done |
| POST | `/users/two-factor/setup` | private | `userHandler.SetupTwoFactor` | done |
| POST | `/users/two-factor/enable` | private | `userHandler.EnableTwoFactor` | done |
| POST | `/users/two-factor/disable` | private | `userHandler.DisableTwoFactor` | done |
| PUT | `/users/attributes` | private | `userHandler.SetAttributes` | done |
| PUT | `/users/email-mfa` | private | `userHandler.UpdateEmailMFA` | done |
| POST | `/users/srp/setup` | private | `userHandler.SetupSRP` | done |
| POST | `/users/srp/complete` | private | `userHandler.CompleteSRPSetup` | done |
| POST | `/users/srp/update` | private | `userHandler.UpdateSrpAndKeyAttributes` | done |
| GET | `/users/srp/attributes` | public | `userHandler.GetSRPAttributes` | done |
| POST | `/users/srp/verify-session` | public | `userHandler.VerifySRPSession` | done |
| POST | `/users/srp/create-session` | public | `userHandler.CreateSRPSession` | done |
| PUT | `/users/recovery-key` | private | `userHandler.SetRecoveryKey` | done |
| GET | `/users/public-key` | private | `userHandler.GetPublicKey` | done |
| POST | `/users/public-keys` | private | `userHandler.GetPublicKeys` | done |
| GET | `/users/session-validity/v2` | private | `userHandler.GetSessionValidityV2` | done |
| POST | `/users/event` | private | `userHandler.ReportEvent` | done |
| POST | `/users/logout` | private | `userHandler.Logout` | done |
| GET | `/users/payment-token` | private | `userHandler.GetPaymentToken` | done |
| GET | `/users/families-token` | private | `userHandler.GetFamiliesToken` | done |
| GET | `/users/accounts-token` | private | `userHandler.GetAccountsToken` | done |
| GET | `/users/details/v2` | private | `userHandler.GetDetailsV2` | done |
| GET | `/users/deletion-summary` | private | `userHandler.GetAccountDeletionSummary` | done |
| GET | `/users/locker-usage` | storage | `userHandler.GetLockerUsage` | done |
| POST | `/users/change-email` | private | `userHandler.ChangeEmail` | done |
| GET | `/users/sessions` | private | `userHandler.GetActiveSessions` | done |
| DELETE | `/users/session` | private | `userHandler.TerminateSession` | done |
| GET | `/users/delete-challenge` | private | `userHandler.GetDeleteChallenge` | done |
| DELETE | `/users/delete` | private | `userHandler.DeleteUser` | done |
| GET | `/users/recover-account` | public | `userHandler.SelfAccountRecovery` | done |
| POST | `/users/recover-account/validate` | public | `userHandler.ValidateSelfAccountRecovery` | done |
| POST | `/users/recover-account` | public | `userHandler.RecoverSelfAccount` | done |
| GET | `/passkeys` | accounts JWT | `passkeysHandler.GetPasskeys` | done |
| PATCH | `/passkeys/{passkeyID}` | accounts JWT | `passkeysHandler.RenamePasskey` | done |
| DELETE | `/passkeys/{passkeyID}` | accounts JWT | `passkeysHandler.DeletePasskey` | done |
| POST | `/passkeys/registration/begin` | accounts JWT | `passkeysHandler.BeginRegistration` | done |
| POST | `/passkeys/registration/finish` | accounts JWT | `passkeysHandler.FinishRegistration` | done |
| POST | `/collections` | storage | `collectionHandler.Create` | done |
| GET | `/collection-actions/pending-remove` | storage | `collectionActionsHandler.ListPendingRemove` | done |
| GET | `/collection-actions/delete-suggestions` | storage | `collectionActionsHandler.ListDeleteSuggestions` | done |
| POST | `/collection-actions/reject-delete-suggestions` | storage | `collectionActionsHandler.RejectDeleteSuggestions` | done |
| GET | `/collections/{collectionID}` | storage | `collectionHandler.GetCollectionByID` | done |
| GET | `/collections` | storage | `collectionHandler.Get` | done |
| GET | `/collections/v2` | storage | `collectionHandler.GetV2` | done |
| GET | `/collections/v3` | storage | `collectionHandler.GetWithLimit` | done |
| POST | `/collections/share` | storage | `collectionHandler.Share` | done |
| POST | `/collections/share/batch` | storage | `collectionHandler.BatchShare` | done |
| POST | `/collections/share/bulk` | storage | `collectionHandler.BulkShare` | done |
| POST | `/collections/join-link` | storage | `collectionHandler.JoinLink` | done |
| POST | `/collections/share-url` | storage | `collectionHandler.ShareURL` | done |
| PUT | `/collections/share-url` | storage | `collectionHandler.UpdateShareURL` | done |
| DELETE | `/collections/share-url/{collectionID}` | storage | `collectionHandler.UnShareURL` | done |
| POST | `/collections/unshare` | storage | `collectionHandler.UnShare` | done |
| POST | `/collections/unshare/bulk` | storage | `collectionHandler.BulkUnShare` | done |
| POST | `/collections/leave/{collectionID}` | storage | `collectionHandler.Leave` | done |
| POST | `/collections/add-files` | storage | `collectionHandler.AddFiles` | done |
| POST | `/collections/move-files` | storage | `collectionHandler.MoveFiles` | done |
| POST | `/collections/restore-files` | storage | `collectionHandler.RestoreFiles` | done |
| POST | `/collections/v3/remove-files` | storage | `collectionHandler.RemoveFilesV3` | done |
| POST | `/collections/suggest-delete` | storage | `collectionHandler.SuggestDeleteInSharedCollection` | done |
| GET | `/collections/v2/diff` | storage | `collectionHandler.GetDiffV2` | done |
| GET | `/collections/file` | storage | `collectionHandler.GetFile` | done |
| GET | `/collections/sharees` | storage | `collectionHandler.GetSharees` | done |
| DELETE | `/collections/v3/{collectionID}` | storage | `collectionHandler.TrashV3` | done |
| POST | `/collections/rename` | storage | `collectionHandler.Rename` | done |
| PUT | `/collections/magic-metadata` | storage | `collectionHandler.PrivateMagicMetadataUpdate` | done |
| PUT | `/collections/public-magic-metadata` | storage | `collectionHandler.PublicMagicMetadataUpdate` | done |
| PUT | `/collections/sharee-magic-metadata` | storage | `collectionHandler.ShareeMagicMetadataUpdate` | done |
| GET | `/file-link/info` | file-link token | `fileHandler.LinkInfo` | done |
| GET | `/file-link/pass-info` | file-link token | `fileHandler.PasswordInfo` | done |
| GET | `/file-link/thumbnail` | file-link token | `fileHandler.LinkThumbnail` | done |
| GET | `/file-link/thumbnail/v3` | file-link token | `fileHandler.LinkThumbnailURLV3` | done |
| GET | `/file-link/file` | file-link token | `fileHandler.LinkFile` | done |
| GET | `/file-link/file/v3` | file-link token | `fileHandler.LinkFileURLV3` | done |
| POST | `/file-link/verify-password` | file-link token | `fileHandler.VerifyPassword` | done |
| GET | `/public-collection/files/preview/{fileID}` | public-collection token | `publicCollectionHandler.GetThumbnail` | done |
| GET | `/public-collection/files/thumbnail/v3/{fileID}` | public-collection token | `publicCollectionHandler.GetThumbnailURLV3` | done |
| GET | `/public-collection/files/download/{fileID}` | public-collection token | `publicCollectionHandler.GetFile` | done |
| GET | `/public-collection/files/download/v3/{fileID}` | public-collection token | `publicCollectionHandler.GetFileURLV3` | done |
| GET | `/public-collection/files/data/fetch` | public-collection token | `publicCollectionHandler.GetFileData` | done |
| GET | `/public-collection/files/data/preview` | public-collection token | `publicCollectionHandler.GetPreviewURL` | done |
| GET | `/public-collection/diff` | public-collection token | `publicCollectionHandler.GetDiff` | done |
| GET | `/public-collection/info` | public-collection token | `publicCollectionHandler.GetCollection` | done |
| POST | `/public-collection/upload-url` | public-collection token | `publicCollectionHandler.GetUploadURLV2` | done |
| POST | `/public-collection/multipart-upload-url` | public-collection token | `publicCollectionHandler.GetMultipartUploadURLV2` | done |
| POST | `/public-collection/file` | public-collection token | `publicCollectionHandler.CreateFile` | done |
| POST | `/public-collection/verify-password` | public-collection token | `publicCollectionHandler.VerifyPassword` | done |
| GET | `/public-collection/social/diff` | public-collection token | `publicSocialHandler.SocialDiff` | done |
| GET | `/public-collection/comments/diff` | public-collection token | `publicSocialHandler.CommentDiff` | done |
| POST | `/public-collection/comments` | public-collection token | `publicSocialHandler.CreateComment` | done |
| PUT | `/public-collection/comments/{commentID}` | public-collection token | `publicSocialHandler.UpdateComment` | done |
| DELETE | `/public-collection/comments/{commentID}` | public-collection token | `publicSocialHandler.DeleteComment` | done |
| GET | `/public-collection/reactions/diff` | public-collection token | `publicSocialHandler.ReactionDiff` | done |
| POST | `/public-collection/reactions` | public-collection token | `publicSocialHandler.CreateReaction` | done |
| DELETE | `/public-collection/reactions/{reactionID}` | public-collection token | `publicSocialHandler.DeleteReaction` | done |
| GET | `/public-collection/participants/masked-emails` | public-collection token | `publicSocialHandler.Participants` | done |
| GET | `/public-collection/anon-profiles` | public-collection token | `publicSocialHandler.AnonProfiles` | done |
| POST | `/public-collection/anon-identity` | public-collection token | `publicSocialHandler.CreateAnonIdentity` | done |
| POST | `/memory-share` | storage | `memoryShareHandler.Create` | done |
| GET | `/memory-share` | storage | `memoryShareHandler.List` | done |
| GET | `/memory-share/{shareID}` | storage | `memoryShareHandler.GetByID` | done |
| DELETE | `/memory-share/{shareID}` | storage | `memoryShareHandler.Delete` | done |
| GET | `/public-memory/info` | public-memory token | `publicMemoryShareHandler.GetInfo` | done |
| GET | `/public-memory/files` | public-memory token | `publicMemoryShareHandler.GetFiles` | done |
| GET | `/public-memory/files/preview/{fileID}` | public-memory token | `publicMemoryShareHandler.GetThumbnail` | done |
| GET | `/public-memory/files/thumbnail/v3/{fileID}` | public-memory token | `publicMemoryShareHandler.GetThumbnailURLV3` | done |
| GET | `/public-memory/files/download/{fileID}` | public-memory token | `publicMemoryShareHandler.GetFile` | done |
| GET | `/public-memory/files/download/v3/{fileID}` | public-memory token | `publicMemoryShareHandler.GetFileURLV3` | done |
| GET | `/public-memory/file-data` | public-memory token | `publicMemoryShareHandler.GetFileData` | done |
| GET | `/public-memory/files/data/preview` | public-memory token | `publicMemoryShareHandler.GetPreviewURL` | done |
| POST | `/cast/device-info` | public | `castHandler.RegisterDevice` | done |
| GET | `/cast/device-info` | storage | `castHandler.GetAllDevices` | done |
| DELETE | `/cast/device-info/{deviceID}` | storage | `castHandler.DeleteDevice` | done |
| POST | `/cast/device-info/` | public | `castHandler.RegisterDevice` | done |
| GET | `/cast/device-info/{deviceCode}` | storage | `castHandler.GetDeviceInfo` | done |
| GET | `/cast/cast-data/{deviceCode}` | public | `castHandler.GetCastData` | done |
| POST | `/cast/cast-data` | storage | `castHandler.InsertCastData` | done |
| POST | `/cast/cast-data/` | storage | `castHandler.InsertCastData` | done |
| DELETE | `/cast/revoke-all-tokens` | storage | `castHandler.RevokeAllToken` | done |
| DELETE | `/cast/revoke-all-tokens/` | storage | `castHandler.RevokeAllToken` | done |
| GET | `/cast/files/preview/{fileID}` | cast token | `castHandler.GetThumbnail` | done |
| GET | `/cast/files/thumbnail/v3/{fileID}` | cast token | `castHandler.GetThumbnailURLV3` | done |
| GET | `/cast/files/download/{fileID}` | cast token | `castHandler.GetFile` | done |
| GET | `/cast/files/download/v3/{fileID}` | cast token | `castHandler.GetFileURLV3` | done |
| GET | `/cast/diff` | cast token | `castHandler.GetDiff` | done |
| GET | `/cast/info` | cast token | `castHandler.GetCollection` | done |
| GET | `/family/invite-info/{token}` | public | `familyHandler.GetInviteInfo` | done |
| POST | `/family/accept-invite` | public | `familyHandler.AcceptInvite` | done |
| DELETE | `/family/leave` | private | `familyHandler.Leave` | done |
| POST | `/family/create` | family JWT | `familyHandler.CreateFamily` | done |
| POST | `/family/add-member` | family JWT | `familyHandler.InviteMember` | done |
| GET | `/family/members` | family JWT | `familyHandler.FetchMembers` | done |
| DELETE | `/family/remove-member/{id}` | family JWT | `familyHandler.RemoveMember` | done |
| DELETE | `/family/revoke-invite/{id}` | family JWT | `familyHandler.RevokeInvite` | done |
| POST | `/family/modify-storage` | family JWT | `familyHandler.ModifyStorageLimit` | done |
| POST | `/emergency-contacts/add` | private | `emergencyHandler.AddContact` | done |
| GET | `/emergency-contacts/info` | private | `emergencyHandler.GetInfo` | done |
| POST | `/emergency-contacts/update` | private | `emergencyHandler.UpdateContact` | done |
| POST | `/emergency-contacts/update-recovery-notice` | private | `emergencyHandler.UpdateRecoveryNotice` | done |
| POST | `/emergency-contacts/start-recovery` | private | `emergencyHandler.StartRecovery` | done |
| POST | `/emergency-contacts/stop-recovery` | private | `emergencyHandler.StopRecovery` | done |
| POST | `/emergency-contacts/reject-recovery` | private | `emergencyHandler.RejectRecovery` | done |
| POST | `/emergency-contacts/approve-recovery` | private | `emergencyHandler.ApproveRecovery` | done |
| GET | `/emergency-contacts/recovery-info/{id}` | private | `emergencyHandler.GetRecoveryInfo` | done |
| POST | `/emergency-contacts/init-change-password` | private | `emergencyHandler.InitChangePassword` | done |
| POST | `/emergency-contacts/change-password` | private | `emergencyHandler.ChangePassword` | done |
| POST | `/legacy-kits` | private | `legacyKitHandler.Create` | done |
| GET | `/legacy-kits` | private | `legacyKitHandler.List` | done |
| GET | `/legacy-kits/{id}/download` | private | `legacyKitHandler.DownloadContent` | done |
| GET | `/legacy-kits/{id}/download-content` | private | `legacyKitHandler.DownloadContent` | done |
| GET | `/legacy-kits/{id}/recovery-session` | private | `legacyKitHandler.OwnerRecoverySession` | done |
| POST | `/legacy-kits/update-recovery-notice` | private | `legacyKitHandler.UpdateRecoveryNotice` | done |
| POST | `/legacy-kits/block-recovery` | private | `legacyKitHandler.BlockRecovery` | done |
| DELETE | `/legacy-kits/{id}` | private | `legacyKitHandler.Delete` | done |
| POST | `/legacy-kits/recovery/challenge` | public | `legacyKitHandler.CreateChallenge` | done |
| POST | `/legacy-kits/recovery/open` | public | `legacyKitHandler.OpenRecovery` | done |
| POST | `/legacy-kits/recovery/session` | public | `legacyKitHandler.Session` | done |
| POST | `/legacy-kits/recovery/info` | public | `legacyKitHandler.RecoveryInfo` | done |
| POST | `/legacy-kits/recovery/init-change-password` | public | `legacyKitHandler.InitChangePassword` | done |
| POST | `/legacy-kits/recovery/change-password` | public | `legacyKitHandler.ChangePassword` | done |
| GET | `/billing/plans/v2` | public | `billingHandler.GetPlansV2` | done |
| GET | `/billing/user-plans` | private | `billingHandler.GetUserPlans` | done |
| GET | `/billing/subscription` | private | `billingHandler.GetSubscription` | done |
| POST | `/billing/verify-subscription` | private | `billingHandler.VerifySubscription` | done |
| POST | `/billing/notify/android` | public | `billingHandler.AndroidNotificationHandler` | stub |
| POST | `/billing/notify/ios` | public | `billingHandler.IOSNotificationHandler` | stub |
| POST | `/billing/notify/stripe` | public | `billingHandler.StripeINNotificationHandler` | stub |
| POST | `/billing/notify/stripe/us` | public | `billingHandler.StripeUSNotificationHandler` | stub |
| GET | `/billing/stripe/customer-portal` | private | `billingHandler.GetStripeCustomerPortal` | stub |
| POST | `/billing/stripe/cancel-subscription` | private | `billingHandler.StripeCancelSubscription` | stub |
| POST | `/billing/stripe/activate-subscription` | private | `billingHandler.StripeActivateSubscription` | stub |
| GET | `/billing/stripe-account-country` | payment JWT | `billingHandler.GetStripeAccountCountry` | stub |
| GET | `/billing/stripe/checkout-session` | payment JWT | `billingHandler.GetCheckoutSession` | stub |
| POST | `/billing/stripe/update-subscription` | payment JWT | `billingHandler.StripeUpdateSubscription` | stub |
| GET | `/storage-bonus/details` | private | `storageBonusHandler.GetStorageBonusDetails` | done |
| POST | `/storage-bonus/change-code` | private | `storageBonusHandler.UpdateReferralCode` | done |
| GET | `/storage-bonus/referral-view` | private | `storageBonusHandler.GetReferralView` | done |
| POST | `/storage-bonus/referral-claim` | private | `storageBonusHandler.ClaimReferral` | done |
| POST | `/admin/mail` | admin | `adminHandler.SendMail` | stub |
| POST | `/admin/mail/subscribe` | admin | `adminHandler.SubscribeMail` | stub |
| POST | `/admin/mail/unsubscribe` | admin | `adminHandler.UnsubscribeMail` | stub |
| GET | `/admin/listmonk/missing-subscribers/count` | admin | `adminHandler.GetListmonkMissingSubscribersCount` | stub |
| GET | `/admin/users` | admin | `adminHandler.GetUsers` | done |
| GET | `/admin/user` | admin | `adminHandler.GetUser` | done |
| GET | `/admin/user/scheduled-deletions` | admin | `adminHandler.GetScheduledDeletions` | done |
| POST | `/admin/user/disable-2fa` | admin | `adminHandler.DisableTwoFactor` | done |
| POST | `/admin/user/update-referral` | admin | `adminHandler.UpdateReferral` | done |
| POST | `/admin/user/disable-passkeys` | admin | `adminHandler.RemovePasskeys` | done |
| POST | `/admin/user/update-email-mfa` | admin | `adminHandler.UpdateEmailMFA` | done |
| POST | `/admin/user/unblock-storage-warning-login` | admin | `adminHandler.UnblockStorageWarningLogin` | done |
| POST | `/admin/user/add-ott` | admin | `adminHandler.AddOtt` | done |
| POST | `/admin/user/terminate-session` | admin | `adminHandler.TerminateSession` | done |
| POST | `/admin/user/close-family` | admin | `adminHandler.CloseFamily` | done |
| PUT | `/admin/user/change-email` | admin | `adminHandler.ChangeEmail` | done |
| DELETE | `/admin/user/delete` | admin | `adminHandler.DeleteUser` | done |
| POST | `/admin/user/recover` | admin | `adminHandler.RecoverAccount` | done |
| POST | `/admin/user/update-flag` | admin | `adminHandler.UpdateFeatureFlag` | done |
| GET | `/admin/email-hash` | admin | `adminHandler.GetEmailHash` | done |
| POST | `/admin/emails-from-hashes` | admin | `adminHandler.GetEmailsFromHashes` | done |
| PUT | `/admin/user/subscription` | admin | `adminHandler.UpdateSubscription` | done |
| POST | `/admin/queue/re-queue` | admin | `adminHandler.ReQueueItem` | stub |
| POST | `/admin/collection/repair-deleted-share` | admin | `adminHandler.RepairDeletedCollectionShare` | stub |
| POST | `/admin/user/init-file-counts` | admin | `adminHandler.InitializeFileCounts` | stub |
| POST | `/admin/user/bonus` | admin | `adminHandler.UpdateBonus` | done |
| POST | `/user-entity/key` | storage | `userEntityHandler.CreateKey` | done |
| POST | `/user-entity/key/ensure` | storage | `userEntityHandler.EnsureKey` | done |
| GET | `/user-entity/key` | storage | `userEntityHandler.GetKey` | done |
| POST | `/user-entity/entity` | storage | `userEntityHandler.CreateEntity` | done |
| PUT | `/user-entity/entity` | storage | `userEntityHandler.UpdateEntity` | done |
| DELETE | `/user-entity/entity` | storage | `userEntityHandler.DeleteEntity` | done |
| GET | `/user-entity/entity/diff` | storage | `userEntityHandler.GetDiff` | done |
| POST | `/contacts` | storage | `contactHandler.Create` | done |
| GET | `/contacts/{id}` | storage | `contactHandler.Get` | done |
| GET | `/contacts/diff` | storage | `contactHandler.GetDiff` | done |
| PUT | `/contacts/{id}` | storage | `contactHandler.Update` | done |
| DELETE | `/contacts/{id}` | storage | `contactHandler.Delete` | done |
| POST | `/attachments/{type}/upload-url` | storage | `contactHandler.GetAttachmentUploadURL` | done |
| GET | `/attachments/{type}/{attachmentID}` | storage | `contactHandler.GetAttachment` | done |
| PUT | `/contacts/{id}/attachments/{type}` | storage | `contactHandler.AttachContactAttachment` | done |
| GET | `/contacts/{id}/attachments/{type}` | storage | `contactHandler.GetCurrentContactAttachment` | done |
| DELETE | `/contacts/{id}/attachments/{type}` | storage | `contactHandler.DeleteContactAttachment` | done |
| POST | `/contacts/{id}/profile-picture/upload-url` | storage | `contactHandler.GetProfilePictureUploadURL` | done |
| PUT | `/contacts/{id}/profile-picture` | storage | `contactHandler.AttachProfilePicture` | done |
| GET | `/contacts/{id}/profile-picture` | storage | `contactHandler.GetProfilePicture` | done |
| DELETE | `/contacts/{id}/profile-picture` | storage | `contactHandler.DeleteProfilePicture` | done |
| POST | `/authenticator/key` | private | `authenticatorHandler.CreateKey` | done |
| GET | `/authenticator/key` | private | `authenticatorHandler.GetKey` | done |
| POST | `/authenticator/entity` | private | `authenticatorHandler.CreateEntity` | done |
| PUT | `/authenticator/entity` | private | `authenticatorHandler.UpdateEntity` | done |
| DELETE | `/authenticator/entity` | private | `authenticatorHandler.DeleteEntity` | done |
| GET | `/authenticator/entity/diff` | private | `authenticatorHandler.GetDiff` | done |
| POST | `/remote-store/update` | private | `remoteStoreHandler.InsertOrUpdate` | done |
| DELETE | `/remote-store/{key}` | private | `remoteStoreHandler.RemoveKey` | done |
| GET | `/remote-store` | private | `remoteStoreHandler.GetKey` | done |
| GET | `/remote-store/feature-flags` | private | `remoteStoreHandler.GetFeatureFlags` | done |
| GET | `/custom-domain` | public | `remoteStoreHandler.CheckDomain` | done |
| POST | `/push/token` | private | `pushHandler.AddToken` | done |
| POST | `/discount/claim` | public | `discountCouponHandler.ClaimCoupon` | done |
| POST | `/admin/discount/add-coupons` | admin | `discountCouponHandler.AddCoupons` | done |

## Photovault-only routes

| Method | Path | Auth | Purpose | Status |
| --- | --- | --- | --- | --- |
| GET | `/healthz` | public | Liveness + database check (Docker `HEALTHCHECK`) | done |
| GET | `/metrics` | public (block at the proxy) | Prometheus metrics | done |
| PUT, GET, HEAD, POST | `/storage/{*key}` | signed URL | Local-backend uploads, multipart parts and completion, downloads | done |
