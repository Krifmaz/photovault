import { styled } from "@mui/material";
import React from "react";
import { enteWordmarkPaths, enteWordmarkViewBox } from "./ente-wordmark";
// PHOTOVAULT: branding - the Photovault apps show the Photovault wordmark.
import { appName } from "ente-base/app";
import { PhotovaultLogo } from "./photovault/PhotovaultLogo";
import { isPhotovaultApp } from "./utils/photovault/encore-tokens";

interface EnteLogoProps {
    height?: number;
}

export const EnteLogo: React.FC<EnteLogoProps> = ({ height }) =>
    // PHOTOVAULT: branding
    isPhotovaultApp(appName) ? (
        <PhotovaultLogo height={height} />
    ) : (
        <svg
            height={height ?? 18}
            viewBox={enteWordmarkViewBox}
            xmlns="http://www.w3.org/2000/svg"
        >
            {enteWordmarkPaths.map((d, index) => (
                <path key={index} d={d} fill="currentColor" />
            ))}
        </svg>
    );

// Remove the inline SVG's baseline gap.
export const EnteLogoBox = styled("div")`
    line-height: 0;
`;
