import { PasskeyVerificationPresentation } from "@/components/auth/PasskeyVerificationForm";
import { PhotosAuthShell } from "@/components/PhotosAuthShell";
import { ServerField } from "@/components/photovault/ServerField"; // PHOTOVAULT: connect
import type { AuthPageConfig } from "ente-accounts/components/auth/AuthPageProvider";
import { decryptBox } from "ente-photos-wasm";

export const authPageConfig: AuthPageConfig = {
    decryptBox,
    recoveryKeyMnemonic: async () => {
        const { recoveryKeyMnemonic } = await import("./services/recovery-key");
        return recoveryKeyMnemonic();
    },
    Shell: PhotosAuthShell,
    recoveryKeyCloseDestination: "/gallery", // PHOTOVAULT: strip - no plan page
    passkeyPresentation: PasskeyVerificationPresentation,
    ServerField, // PHOTOVAULT: connect - also on /login and /signup
};
