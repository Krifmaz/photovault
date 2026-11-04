import type { ServerCheck } from "ente-base/photovault/server"; // PHOTOVAULT: connect
import type { SessionStorageCrypto } from "ente-base/session-storage";
import {
    createContext,
    useContext,
    type ComponentType,
    type PropsWithChildren,
} from "react";
import type { VerifyingPasskeyPresentationProps } from "../LoginComponents";
import type { AuthPageShellProps } from "./AuthPageShell";

export interface AuthLoginFrameProps extends PropsWithChildren {
    onHostChanged: () => void;
}

export interface AuthPageConfig {
    decryptBox: SessionStorageCrypto["decryptBox"];
    LoginFrame?: ComponentType<AuthLoginFrameProps>;
    keepLoginLoadingOnRedirect?: boolean;
    recoveryKeyCloseDestination?: string;
    recoveryKeyMnemonic?: () => Promise<string>;
    passkeyPresentation?: ComponentType<VerifyingPasskeyPresentationProps>;
    Shell: ComponentType<Pick<AuthPageShellProps, "children" | "contentWidth">>;
    /**
     * PHOTOVAULT: connect - the server field shown below the login and
     * sign-up forms (in place of their bare host line), as on the photos
     * app's index page. Unset in Ente's other apps.
     */
    ServerField?: ComponentType<{
        onChange: () => void;
        onCheck?: (check: ServerCheck) => void;
    }>;
}

const AuthPageContext = createContext<AuthPageConfig | undefined>(undefined);

export const AuthPageProvider = AuthPageContext.Provider;

export const useAuthPageConfig = (): AuthPageConfig => {
    const config = useContext(AuthPageContext);
    if (!config)
        throw new Error("AuthPageProvider is required for account pages");
    return config;
};
