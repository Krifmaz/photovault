import "@fontsource-variable/inter";
import "@fontsource-variable/outfit";
import "@fontsource/itim";
import { CssBaseline, GlobalStyles } from "@mui/material";
import { ThemeProvider } from "@mui/material/styles";
import { CustomHead } from "ente-base/components/Head";
import { useSetupLogs } from "ente-base/components/utils/hooks-app";
// PHOTOVAULT: theme - memories use the Photovault (Encore) theme.
import { photosTheme } from "ente-base/components/utils/theme";
import type { AppProps } from "next/app";
import React from "react";

const memoriesAppStyle = { "--font-itim": "'Itim'" } as React.CSSProperties;
const memoriesAppTitle = "Memories, shared with you";
// PHOTOVAULT: branding
const memoriesAppDescription = "Tap to relive with Photovault";

// PHOTOVAULT: strip - no Ente-hosted preview (og:image) metadata.
function MemoriesAppHead() {
    return (
        <CustomHead
            title={memoriesAppTitle}
            description={memoriesAppDescription}
        />
    );
}

const App: React.FC<AppProps> = ({ Component, pageProps }) => {
    useSetupLogs({ disableDiskLogs: true });

    return (
        <div style={memoriesAppStyle}>
            <ThemeProvider
                theme={photosTheme} // PHOTOVAULT: theme
                defaultMode="system"
                // Avoid persisting a manual override; always follow the system theme.
                storageManager={null}
            >
                <MemoriesAppHead />
                <CssBaseline enableColorScheme />
                <GlobalStyles
                    styles={{
                        html: { height: "100%" },
                        body: {
                            height: "100%",
                            overflow: "hidden",
                            overscrollBehavior: "none",
                        },
                        "#__next": { height: "100%", overflow: "hidden" },
                    }}
                />
                <Component {...pageProps} />
            </ThemeProvider>
        </div>
    );
};

export default App;
