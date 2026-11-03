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
| GET | `/ping` | public | `(inline)` | planned |
| GET | `/fire/db-m-ping` | public | `(inline)` | planned |
| POST | `/events` | public | `eventHandler.Create` | planned |
| POST | `/events/user` | private | `eventHandler.CreateForUser` | planned |
| GET | `/files/upload-eligibility` | storage | `fileHandler.ValidateUploadEligibility` | planned |
| GET | `/files/upload-urls` | storage | `fileHandler.GetUploadURLs` | planned |
| GET | `/files/multipart-upload-urls` | storage | `fileHandler.GetMultipartUploadURLs` | planned |
| POST | `/files/upload-url` | storage | `fileHandler.GetUploadURLV2` | planned |
| POST | `/files/multipart-upload-url` | storage | `fileHandler.GetMultipartUploadURLV2` | planned |
| GET | `/files/download/{fileID}` | storage | `fileHandler.Get` | planned |
| GET | `/files/download/v2/{fileID}` | storage | `fileHandler.GetURL` | planned |
| GET | `/files/download/v3/{fileID}` | storage | `fileHandler.GetURLV3` | planned |
| GET | `/files/preview/{fileID}` | storage | `fileHandler.GetThumbnail` | planned |
| GET | `/files/preview/v2/{fileID}` | storage | `fileHandler.GetThumbnailURL` | planned |
| GET | `/files/thumbnail/v3/{fileID}` | storage | `fileHandler.GetThumbnailURLV3` | planned |
| POST | `/files/share-url` | storage | `fileHandler.ShareUrl` | planned |
| GET | `/files/share-url` | storage | `fileHandler.GetUrls` | planned |
| PUT | `/files/share-url` | storage | `fileHandler.UpdateFileURL` | planned |
| DELETE | `/files/share-url/{fileID}` | storage | `fileHandler.DisableUrl` | planned |
| GET | `/files/share-urls/` | storage | `fileHandler.GetUrls` | planned |
| PUT | `/files/data` | storage | `fileHandler.PutFileData` | planned |
| PUT | `/files/video-data` | storage | `fileHandler.PutVideoData` | planned |
| POST | `/files/data/status-diff` | storage | `fileHandler.FileDataStatusDiff` | planned |
| POST | `/files/data/fetch` | storage | `fileHandler.GetFilesData` | planned |
| GET | `/files/data/fetch` | storage | `fileHandler.GetFileData` | planned |
| GET | `/files/data/preview-upload-url` | storage | `fileHandler.GetPreviewUploadURL` | planned |
| POST | `/files/data/preview-upload-url` | storage | `fileHandler.GetPreviewUploadURLV2` | planned |
| POST | `/files/data/multipart-preview-upload-url` | storage | `fileHandler.GetMultipartPreviewUploadURL` | planned |
| GET | `/files/data/preview` | storage | `fileHandler.GetPreviewURL` | planned |
| POST | `/files` | storage | `fileHandler.CreateOrUpdate` | planned |
| POST | `/files/meta` | storage | `fileHandler.CreateMetaFile` | planned |
| POST | `/files/copy` | storage | `fileHandler.CopyFiles` | planned |
| PUT | `/files/update` | storage | `fileHandler.Update` | planned |
| POST | `/files/trash` | storage | `fileHandler.Trash` | planned |
| POST | `/files/size` | storage | `fileHandler.GetSize` | planned |
| POST | `/files/info` | storage | `fileHandler.GetInfo` | planned |
| GET | `/files/duplicates` | storage | `fileHandler.GetDuplicates` | planned |
| PUT | `/files/thumbnail` | storage | `fileHandler.UpdateThumbnail` | planned |
| PUT | `/files/magic-metadata` | storage | `fileHandler.UpdateMagicMetadata` | planned |
| PUT | `/files/public-magic-metadata` | storage | `fileHandler.UpdatePublicMagicMetadata` | planned |
| GET | `/files/count` | public | `fileHandler.GetTotalFileCount` | planned |
| POST | `/paste/create` | public | `pasteHandler.Create` | planned |
| POST | `/paste/guard` | public | `pasteHandler.Guard` | planned |
| POST | `/paste/consume` | public | `pasteHandler.Consume` | planned |
| GET | `/trash/v2/diff` | storage | `trashHandler.GetDiffV2` | planned |
| POST | `/trash/delete` | storage | `trashHandler.Delete` | planned |
| POST | `/trash/empty` | storage | `trashHandler.Empty` | planned |
| GET | `/comments/diff` | storage | `commentsHandler.Diff` | planned |
| POST | `/comments` | storage | `commentsHandler.Create` | planned |
| PUT | `/comments/{commentID}` | storage | `commentsHandler.Update` | planned |
| DELETE | `/comments/{commentID}` | storage | `commentsHandler.Delete` | planned |
| GET | `/reactions/diff` | storage | `reactionsHandler.Diff` | planned |
| PUT | `/reactions` | storage | `reactionsHandler.Upsert` | planned |
| DELETE | `/reactions/{reactionID}` | storage | `reactionsHandler.Delete` | planned |
| GET | `/social/diff` | storage | `socialHandler.UnifiedDiff` | planned |
| GET | `/social/anon-profiles` | storage | `socialHandler.AnonProfiles` | planned |
| GET | `/comments-reactions/updated-at` | storage | `socialHandler.LatestUpdates` | planned |
| POST | `/users/ott` | public | `userHandler.SendOTT` | planned |
| POST | `/users/verify-email` | public | `userHandler.VerifyEmail` | planned |
| POST | `/users/two-factor/verify` | public | `userHandler.VerifyTwoFactor` | planned |
| GET | `/users/two-factor/recover` | public | `userHandler.RecoverTwoFactor` | planned |
| POST | `/users/two-factor/remove` | public | `userHandler.RemoveTwoFactor` | planned |
| POST | `/users/two-factor/passkeys/begin` | public | `userHandler.BeginPasskeyAuthenticationCeremony` | planned |
| POST | `/users/two-factor/passkeys/finish` | public | `userHandler.FinishPasskeyAuthenticationCeremony` | planned |
| GET | `/users/two-factor/passkeys/get-token` | public | `userHandler.GetTokenForPasskeySession` | planned |
| GET | `/users/two-factor/recovery-status` | private | `userHandler.GetTwoFactorRecoveryStatus` | planned |
| POST | `/users/two-factor/passkeys/configure-recovery` | private | `userHandler.ConfigurePasskeyRecovery` | planned |
| GET | `/users/two-factor/status` | private | `userHandler.GetTwoFactorStatus` | planned |
| POST | `/users/two-factor/setup` | private | `userHandler.SetupTwoFactor` | planned |
| POST | `/users/two-factor/enable` | private | `userHandler.EnableTwoFactor` | planned |
| POST | `/users/two-factor/disable` | private | `userHandler.DisableTwoFactor` | planned |
| PUT | `/users/attributes` | private | `userHandler.SetAttributes` | planned |
| PUT | `/users/email-mfa` | private | `userHandler.UpdateEmailMFA` | planned |
| POST | `/users/srp/setup` | private | `userHandler.SetupSRP` | planned |
| POST | `/users/srp/complete` | private | `userHandler.CompleteSRPSetup` | planned |
| POST | `/users/srp/update` | private | `userHandler.UpdateSrpAndKeyAttributes` | planned |
| GET | `/users/srp/attributes` | public | `userHandler.GetSRPAttributes` | planned |
| POST | `/users/srp/verify-session` | public | `userHandler.VerifySRPSession` | planned |
| POST | `/users/srp/create-session` | public | `userHandler.CreateSRPSession` | planned |
| PUT | `/users/recovery-key` | private | `userHandler.SetRecoveryKey` | planned |
| GET | `/users/public-key` | private | `userHandler.GetPublicKey` | planned |
| POST | `/users/public-keys` | private | `userHandler.GetPublicKeys` | planned |
| GET | `/users/session-validity/v2` | private | `userHandler.GetSessionValidityV2` | planned |
| POST | `/users/event` | private | `userHandler.ReportEvent` | planned |
| POST | `/users/logout` | private | `userHandler.Logout` | planned |
| GET | `/users/payment-token` | private | `userHandler.GetPaymentToken` | planned |
| GET | `/users/families-token` | private | `userHandler.GetFamiliesToken` | planned |
| GET | `/users/accounts-token` | private | `userHandler.GetAccountsToken` | planned |
| GET | `/users/details/v2` | private | `userHandler.GetDetailsV2` | planned |
| GET | `/users/deletion-summary` | private | `userHandler.GetAccountDeletionSummary` | planned |
| GET | `/users/locker-usage` | storage | `userHandler.GetLockerUsage` | planned |
| POST | `/users/change-email` | private | `userHandler.ChangeEmail` | planned |
| GET | `/users/sessions` | private | `userHandler.GetActiveSessions` | planned |
| DELETE | `/users/session` | private | `userHandler.TerminateSession` | planned |
| GET | `/users/delete-challenge` | private | `userHandler.GetDeleteChallenge` | planned |
| DELETE | `/users/delete` | private | `userHandler.DeleteUser` | planned |
| GET | `/users/recover-account` | public | `userHandler.SelfAccountRecovery` | planned |
| POST | `/users/recover-account/validate` | public | `userHandler.ValidateSelfAccountRecovery` | planned |
| POST | `/users/recover-account` | public | `userHandler.RecoverSelfAccount` | planned |
| GET | `/passkeys` | accounts JWT | `passkeysHandler.GetPasskeys` | planned |
| PATCH | `/passkeys/{passkeyID}` | accounts JWT | `passkeysHandler.RenamePasskey` | planned |
| DELETE | `/passkeys/{passkeyID}` | accounts JWT | `passkeysHandler.DeletePasskey` | planned |
| POST | `/passkeys/registration/begin` | accounts JWT | `passkeysHandler.BeginRegistration` | planned |
| POST | `/passkeys/registration/finish` | accounts JWT | `passkeysHandler.FinishRegistration` | planned |
| POST | `/collections` | storage | `collectionHandler.Create` | planned |
| GET | `/collection-actions/pending-remove` | storage | `collectionActionsHandler.ListPendingRemove` | planned |
| GET | `/collection-actions/delete-suggestions` | storage | `collectionActionsHandler.ListDeleteSuggestions` | planned |
| POST | `/collection-actions/reject-delete-suggestions` | storage | `collectionActionsHandler.RejectDeleteSuggestions` | planned |
| GET | `/collections/{collectionID}` | storage | `collectionHandler.GetCollectionByID` | planned |
| GET | `/collections` | storage | `collectionHandler.Get` | planned |
| GET | `/collections/v2` | storage | `collectionHandler.GetV2` | planned |
| GET | `/collections/v3` | storage | `collectionHandler.GetWithLimit` | planned |
| POST | `/collections/share` | storage | `collectionHandler.Share` | planned |
| POST | `/collections/share/batch` | storage | `collectionHandler.BatchShare` | planned |
| POST | `/collections/share/bulk` | storage | `collectionHandler.BulkShare` | planned |
| POST | `/collections/join-link` | storage | `collectionHandler.JoinLink` | planned |
| POST | `/collections/share-url` | storage | `collectionHandler.ShareURL` | planned |
| PUT | `/collections/share-url` | storage | `collectionHandler.UpdateShareURL` | planned |
| DELETE | `/collections/share-url/{collectionID}` | storage | `collectionHandler.UnShareURL` | planned |
| POST | `/collections/unshare` | storage | `collectionHandler.UnShare` | planned |
| POST | `/collections/unshare/bulk` | storage | `collectionHandler.BulkUnShare` | planned |
| POST | `/collections/leave/{collectionID}` | storage | `collectionHandler.Leave` | planned |
| POST | `/collections/add-files` | storage | `collectionHandler.AddFiles` | planned |
| POST | `/collections/move-files` | storage | `collectionHandler.MoveFiles` | planned |
| POST | `/collections/restore-files` | storage | `collectionHandler.RestoreFiles` | planned |
| POST | `/collections/v3/remove-files` | storage | `collectionHandler.RemoveFilesV3` | planned |
| POST | `/collections/suggest-delete` | storage | `collectionHandler.SuggestDeleteInSharedCollection` | planned |
| GET | `/collections/v2/diff` | storage | `collectionHandler.GetDiffV2` | planned |
| GET | `/collections/file` | storage | `collectionHandler.GetFile` | planned |
| GET | `/collections/sharees` | storage | `collectionHandler.GetSharees` | planned |
| DELETE | `/collections/v3/{collectionID}` | storage | `collectionHandler.TrashV3` | planned |
| POST | `/collections/rename` | storage | `collectionHandler.Rename` | planned |
| PUT | `/collections/magic-metadata` | storage | `collectionHandler.PrivateMagicMetadataUpdate` | planned |
| PUT | `/collections/public-magic-metadata` | storage | `collectionHandler.PublicMagicMetadataUpdate` | planned |
| PUT | `/collections/sharee-magic-metadata` | storage | `collectionHandler.ShareeMagicMetadataUpdate` | planned |
| GET | `/file-link/info` | file-link token | `fileHandler.LinkInfo` | planned |
| GET | `/file-link/pass-info` | file-link token | `fileHandler.PasswordInfo` | planned |
| GET | `/file-link/thumbnail` | file-link token | `fileHandler.LinkThumbnail` | planned |
| GET | `/file-link/thumbnail/v3` | file-link token | `fileHandler.LinkThumbnailURLV3` | planned |
| GET | `/file-link/file` | file-link token | `fileHandler.LinkFile` | planned |
| GET | `/file-link/file/v3` | file-link token | `fileHandler.LinkFileURLV3` | planned |
| POST | `/file-link/verify-password` | file-link token | `fileHandler.VerifyPassword` | planned |
| GET | `/public-collection/files/preview/{fileID}` | public-collection token | `publicCollectionHandler.GetThumbnail` | planned |
| GET | `/public-collection/files/thumbnail/v3/{fileID}` | public-collection token | `publicCollectionHandler.GetThumbnailURLV3` | planned |
| GET | `/public-collection/files/download/{fileID}` | public-collection token | `publicCollectionHandler.GetFile` | planned |
| GET | `/public-collection/files/download/v3/{fileID}` | public-collection token | `publicCollectionHandler.GetFileURLV3` | planned |
| GET | `/public-collection/files/data/fetch` | public-collection token | `publicCollectionHandler.GetFileData` | planned |
| GET | `/public-collection/files/data/preview` | public-collection token | `publicCollectionHandler.GetPreviewURL` | planned |
| GET | `/public-collection/diff` | public-collection token | `publicCollectionHandler.GetDiff` | planned |
| GET | `/public-collection/info` | public-collection token | `publicCollectionHandler.GetCollection` | planned |
| POST | `/public-collection/upload-url` | public-collection token | `publicCollectionHandler.GetUploadURLV2` | planned |
| POST | `/public-collection/multipart-upload-url` | public-collection token | `publicCollectionHandler.GetMultipartUploadURLV2` | planned |
| POST | `/public-collection/file` | public-collection token | `publicCollectionHandler.CreateFile` | planned |
| POST | `/public-collection/verify-password` | public-collection token | `publicCollectionHandler.VerifyPassword` | planned |
| GET | `/public-collection/social/diff` | public-collection token | `publicSocialHandler.SocialDiff` | planned |
| GET | `/public-collection/comments/diff` | public-collection token | `publicSocialHandler.CommentDiff` | planned |
| POST | `/public-collection/comments` | public-collection token | `publicSocialHandler.CreateComment` | planned |
| PUT | `/public-collection/comments/{commentID}` | public-collection token | `publicSocialHandler.UpdateComment` | planned |
| DELETE | `/public-collection/comments/{commentID}` | public-collection token | `publicSocialHandler.DeleteComment` | planned |
| GET | `/public-collection/reactions/diff` | public-collection token | `publicSocialHandler.ReactionDiff` | planned |
| POST | `/public-collection/reactions` | public-collection token | `publicSocialHandler.CreateReaction` | planned |
| DELETE | `/public-collection/reactions/{reactionID}` | public-collection token | `publicSocialHandler.DeleteReaction` | planned |
| GET | `/public-collection/participants/masked-emails` | public-collection token | `publicSocialHandler.Participants` | planned |
| GET | `/public-collection/anon-profiles` | public-collection token | `publicSocialHandler.AnonProfiles` | planned |
| POST | `/public-collection/anon-identity` | public-collection token | `publicSocialHandler.CreateAnonIdentity` | planned |
| POST | `/memory-share` | storage | `memoryShareHandler.Create` | planned |
| GET | `/memory-share` | storage | `memoryShareHandler.List` | planned |
| GET | `/memory-share/{shareID}` | storage | `memoryShareHandler.GetByID` | planned |
| DELETE | `/memory-share/{shareID}` | storage | `memoryShareHandler.Delete` | planned |
| GET | `/public-memory/info` | public-memory token | `publicMemoryShareHandler.GetInfo` | planned |
| GET | `/public-memory/files` | public-memory token | `publicMemoryShareHandler.GetFiles` | planned |
| GET | `/public-memory/files/preview/{fileID}` | public-memory token | `publicMemoryShareHandler.GetThumbnail` | planned |
| GET | `/public-memory/files/thumbnail/v3/{fileID}` | public-memory token | `publicMemoryShareHandler.GetThumbnailURLV3` | planned |
| GET | `/public-memory/files/download/{fileID}` | public-memory token | `publicMemoryShareHandler.GetFile` | planned |
| GET | `/public-memory/files/download/v3/{fileID}` | public-memory token | `publicMemoryShareHandler.GetFileURLV3` | planned |
| GET | `/public-memory/file-data` | public-memory token | `publicMemoryShareHandler.GetFileData` | planned |
| GET | `/public-memory/files/data/preview` | public-memory token | `publicMemoryShareHandler.GetPreviewURL` | planned |
| POST | `/cast/device-info` | public | `castHandler.RegisterDevice` | planned |
| GET | `/cast/device-info` | storage | `castHandler.GetAllDevices` | planned |
| DELETE | `/cast/device-info/{deviceID}` | storage | `castHandler.DeleteDevice` | planned |
| POST | `/cast/device-info/` | public | `castHandler.RegisterDevice` | planned |
| GET | `/cast/device-info/{deviceCode}` | storage | `castHandler.GetDeviceInfo` | planned |
| GET | `/cast/cast-data/{deviceCode}` | public | `castHandler.GetCastData` | planned |
| POST | `/cast/cast-data` | storage | `castHandler.InsertCastData` | planned |
| POST | `/cast/cast-data/` | storage | `castHandler.InsertCastData` | planned |
| DELETE | `/cast/revoke-all-tokens` | storage | `castHandler.RevokeAllToken` | planned |
| DELETE | `/cast/revoke-all-tokens/` | storage | `castHandler.RevokeAllToken` | planned |
| GET | `/cast/files/preview/{fileID}` | cast token | `castHandler.GetThumbnail` | planned |
| GET | `/cast/files/thumbnail/v3/{fileID}` | cast token | `castHandler.GetThumbnailURLV3` | planned |
| GET | `/cast/files/download/{fileID}` | cast token | `castHandler.GetFile` | planned |
| GET | `/cast/files/download/v3/{fileID}` | cast token | `castHandler.GetFileURLV3` | planned |
| GET | `/cast/diff` | cast token | `castHandler.GetDiff` | planned |
| GET | `/cast/info` | cast token | `castHandler.GetCollection` | planned |
| GET | `/family/invite-info/{token}` | public | `familyHandler.GetInviteInfo` | planned |
| POST | `/family/accept-invite` | public | `familyHandler.AcceptInvite` | planned |
| DELETE | `/family/leave` | private | `familyHandler.Leave` | planned |
| POST | `/family/create` | family JWT | `familyHandler.CreateFamily` | planned |
| POST | `/family/add-member` | family JWT | `familyHandler.InviteMember` | planned |
| GET | `/family/members` | family JWT | `familyHandler.FetchMembers` | planned |
| DELETE | `/family/remove-member/{id}` | family JWT | `familyHandler.RemoveMember` | planned |
| DELETE | `/family/revoke-invite/{id}` | family JWT | `familyHandler.RevokeInvite` | planned |
| POST | `/family/modify-storage` | family JWT | `familyHandler.ModifyStorageLimit` | planned |
| POST | `/emergency-contacts/add` | private | `emergencyHandler.AddContact` | planned |
| GET | `/emergency-contacts/info` | private | `emergencyHandler.GetInfo` | planned |
| POST | `/emergency-contacts/update` | private | `emergencyHandler.UpdateContact` | planned |
| POST | `/emergency-contacts/update-recovery-notice` | private | `emergencyHandler.UpdateRecoveryNotice` | planned |
| POST | `/emergency-contacts/start-recovery` | private | `emergencyHandler.StartRecovery` | planned |
| POST | `/emergency-contacts/stop-recovery` | private | `emergencyHandler.StopRecovery` | planned |
| POST | `/emergency-contacts/reject-recovery` | private | `emergencyHandler.RejectRecovery` | planned |
| POST | `/emergency-contacts/approve-recovery` | private | `emergencyHandler.ApproveRecovery` | planned |
| GET | `/emergency-contacts/recovery-info/{id}` | private | `emergencyHandler.GetRecoveryInfo` | planned |
| POST | `/emergency-contacts/init-change-password` | private | `emergencyHandler.InitChangePassword` | planned |
| POST | `/emergency-contacts/change-password` | private | `emergencyHandler.ChangePassword` | planned |
| POST | `/legacy-kits` | private | `legacyKitHandler.Create` | planned |
| GET | `/legacy-kits` | private | `legacyKitHandler.List` | planned |
| GET | `/legacy-kits/{id}/download` | private | `legacyKitHandler.DownloadContent` | planned |
| GET | `/legacy-kits/{id}/download-content` | private | `legacyKitHandler.DownloadContent` | planned |
| GET | `/legacy-kits/{id}/recovery-session` | private | `legacyKitHandler.OwnerRecoverySession` | planned |
| POST | `/legacy-kits/update-recovery-notice` | private | `legacyKitHandler.UpdateRecoveryNotice` | planned |
| POST | `/legacy-kits/block-recovery` | private | `legacyKitHandler.BlockRecovery` | planned |
| DELETE | `/legacy-kits/{id}` | private | `legacyKitHandler.Delete` | planned |
| POST | `/legacy-kits/recovery/challenge` | public | `legacyKitHandler.CreateChallenge` | planned |
| POST | `/legacy-kits/recovery/open` | public | `legacyKitHandler.OpenRecovery` | planned |
| POST | `/legacy-kits/recovery/session` | public | `legacyKitHandler.Session` | planned |
| POST | `/legacy-kits/recovery/info` | public | `legacyKitHandler.RecoveryInfo` | planned |
| POST | `/legacy-kits/recovery/init-change-password` | public | `legacyKitHandler.InitChangePassword` | planned |
| POST | `/legacy-kits/recovery/change-password` | public | `legacyKitHandler.ChangePassword` | planned |
| GET | `/billing/plans/v2` | public | `billingHandler.GetPlansV2` | planned |
| GET | `/billing/user-plans` | private | `billingHandler.GetUserPlans` | planned |
| GET | `/billing/subscription` | private | `billingHandler.GetSubscription` | planned |
| POST | `/billing/verify-subscription` | private | `billingHandler.VerifySubscription` | planned |
| POST | `/billing/notify/android` | public | `billingHandler.AndroidNotificationHandler` | planned |
| POST | `/billing/notify/ios` | public | `billingHandler.IOSNotificationHandler` | planned |
| POST | `/billing/notify/stripe` | public | `billingHandler.StripeINNotificationHandler` | planned |
| POST | `/billing/notify/stripe/us` | public | `billingHandler.StripeUSNotificationHandler` | planned |
| GET | `/billing/stripe/customer-portal` | private | `billingHandler.GetStripeCustomerPortal` | planned |
| POST | `/billing/stripe/cancel-subscription` | private | `billingHandler.StripeCancelSubscription` | planned |
| POST | `/billing/stripe/activate-subscription` | private | `billingHandler.StripeActivateSubscription` | planned |
| GET | `/billing/stripe-account-country` | payment JWT | `billingHandler.GetStripeAccountCountry` | planned |
| GET | `/billing/stripe/checkout-session` | payment JWT | `billingHandler.GetCheckoutSession` | planned |
| POST | `/billing/stripe/update-subscription` | payment JWT | `billingHandler.StripeUpdateSubscription` | planned |
| GET | `/storage-bonus/details` | private | `storageBonusHandler.GetStorageBonusDetails` | planned |
| POST | `/storage-bonus/change-code` | private | `storageBonusHandler.UpdateReferralCode` | planned |
| GET | `/storage-bonus/referral-view` | private | `storageBonusHandler.GetReferralView` | planned |
| POST | `/storage-bonus/referral-claim` | private | `storageBonusHandler.ClaimReferral` | planned |
| POST | `/admin/mail` | admin | `adminHandler.SendMail` | planned |
| POST | `/admin/mail/subscribe` | admin | `adminHandler.SubscribeMail` | planned |
| POST | `/admin/mail/unsubscribe` | admin | `adminHandler.UnsubscribeMail` | planned |
| GET | `/admin/listmonk/missing-subscribers/count` | admin | `adminHandler.GetListmonkMissingSubscribersCount` | planned |
| GET | `/admin/users` | admin | `adminHandler.GetUsers` | planned |
| GET | `/admin/user` | admin | `adminHandler.GetUser` | planned |
| GET | `/admin/user/scheduled-deletions` | admin | `adminHandler.GetScheduledDeletions` | planned |
| POST | `/admin/user/disable-2fa` | admin | `adminHandler.DisableTwoFactor` | planned |
| POST | `/admin/user/update-referral` | admin | `adminHandler.UpdateReferral` | planned |
| POST | `/admin/user/disable-passkeys` | admin | `adminHandler.RemovePasskeys` | planned |
| POST | `/admin/user/update-email-mfa` | admin | `adminHandler.UpdateEmailMFA` | planned |
| POST | `/admin/user/unblock-storage-warning-login` | admin | `adminHandler.UnblockStorageWarningLogin` | planned |
| POST | `/admin/user/add-ott` | admin | `adminHandler.AddOtt` | planned |
| POST | `/admin/user/terminate-session` | admin | `adminHandler.TerminateSession` | planned |
| POST | `/admin/user/close-family` | admin | `adminHandler.CloseFamily` | planned |
| PUT | `/admin/user/change-email` | admin | `adminHandler.ChangeEmail` | planned |
| DELETE | `/admin/user/delete` | admin | `adminHandler.DeleteUser` | planned |
| POST | `/admin/user/recover` | admin | `adminHandler.RecoverAccount` | planned |
| POST | `/admin/user/update-flag` | admin | `adminHandler.UpdateFeatureFlag` | planned |
| GET | `/admin/email-hash` | admin | `adminHandler.GetEmailHash` | planned |
| POST | `/admin/emails-from-hashes` | admin | `adminHandler.GetEmailsFromHashes` | planned |
| PUT | `/admin/user/subscription` | admin | `adminHandler.UpdateSubscription` | planned |
| POST | `/admin/queue/re-queue` | admin | `adminHandler.ReQueueItem` | planned |
| POST | `/admin/collection/repair-deleted-share` | admin | `adminHandler.RepairDeletedCollectionShare` | planned |
| POST | `/admin/user/init-file-counts` | admin | `adminHandler.InitializeFileCounts` | planned |
| POST | `/admin/user/bonus` | admin | `adminHandler.UpdateBonus` | planned |
| POST | `/user-entity/key` | storage | `userEntityHandler.CreateKey` | planned |
| POST | `/user-entity/key/ensure` | storage | `userEntityHandler.EnsureKey` | planned |
| GET | `/user-entity/key` | storage | `userEntityHandler.GetKey` | planned |
| POST | `/user-entity/entity` | storage | `userEntityHandler.CreateEntity` | planned |
| PUT | `/user-entity/entity` | storage | `userEntityHandler.UpdateEntity` | planned |
| DELETE | `/user-entity/entity` | storage | `userEntityHandler.DeleteEntity` | planned |
| GET | `/user-entity/entity/diff` | storage | `userEntityHandler.GetDiff` | planned |
| POST | `/contacts` | storage | `contactHandler.Create` | planned |
| GET | `/contacts/{id}` | storage | `contactHandler.Get` | planned |
| GET | `/contacts/diff` | storage | `contactHandler.GetDiff` | planned |
| PUT | `/contacts/{id}` | storage | `contactHandler.Update` | planned |
| DELETE | `/contacts/{id}` | storage | `contactHandler.Delete` | planned |
| POST | `/attachments/{type}/upload-url` | storage | `contactHandler.GetAttachmentUploadURL` | planned |
| GET | `/attachments/{type}/{attachmentID}` | storage | `contactHandler.GetAttachment` | planned |
| PUT | `/contacts/{id}/attachments/{type}` | storage | `contactHandler.AttachContactAttachment` | planned |
| GET | `/contacts/{id}/attachments/{type}` | storage | `contactHandler.GetCurrentContactAttachment` | planned |
| DELETE | `/contacts/{id}/attachments/{type}` | storage | `contactHandler.DeleteContactAttachment` | planned |
| POST | `/contacts/{id}/profile-picture/upload-url` | storage | `contactHandler.GetProfilePictureUploadURL` | planned |
| PUT | `/contacts/{id}/profile-picture` | storage | `contactHandler.AttachProfilePicture` | planned |
| GET | `/contacts/{id}/profile-picture` | storage | `contactHandler.GetProfilePicture` | planned |
| DELETE | `/contacts/{id}/profile-picture` | storage | `contactHandler.DeleteProfilePicture` | planned |
| POST | `/authenticator/key` | private | `authenticatorHandler.CreateKey` | planned |
| GET | `/authenticator/key` | private | `authenticatorHandler.GetKey` | planned |
| POST | `/authenticator/entity` | private | `authenticatorHandler.CreateEntity` | planned |
| PUT | `/authenticator/entity` | private | `authenticatorHandler.UpdateEntity` | planned |
| DELETE | `/authenticator/entity` | private | `authenticatorHandler.DeleteEntity` | planned |
| GET | `/authenticator/entity/diff` | private | `authenticatorHandler.GetDiff` | planned |
| POST | `/remote-store/update` | private | `remoteStoreHandler.InsertOrUpdate` | planned |
| DELETE | `/remote-store/{key}` | private | `remoteStoreHandler.RemoveKey` | planned |
| GET | `/remote-store` | private | `remoteStoreHandler.GetKey` | planned |
| GET | `/remote-store/feature-flags` | private | `remoteStoreHandler.GetFeatureFlags` | planned |
| GET | `/custom-domain` | public | `remoteStoreHandler.CheckDomain` | planned |
| POST | `/push/token` | private | `pushHandler.AddToken` | planned |
| POST | `/discount/claim` | public | `discountCouponHandler.ClaimCoupon` | planned |
| POST | `/admin/discount/add-coupons` | admin | `discountCouponHandler.AddCoupons` | planned |

## Photovault-only routes

| Method | Path | Auth | Purpose | Status |
| --- | --- | --- | --- | --- |
| GET | `/healthz` | public | Liveness + database check (Docker `HEALTHCHECK`) | done |
| GET | `/metrics` | public (block at the proxy) | Prometheus metrics | done |
| PUT, GET, HEAD, POST | `/storage/{*key}` | signed URL | Local-backend uploads, multipart parts and completion, downloads | done |
