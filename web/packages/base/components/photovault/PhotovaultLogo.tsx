// PHOTOVAULT: branding (new file).

import React, { useId } from "react";
import { pvWordmarkSVGHTML, pvWordmarkViewBox } from "./wordmark";

interface PhotovaultLogoProps {
    /** Height in px (default 18, the same as Ente's logo). */
    height?: number;
}

/**
 * The Photovault wordmark, a drop-in replacement for Ente's `EnteLogo` with
 * the same props and sizing (height given, width from the viewBox).
 */
export const PhotovaultLogo: React.FC<PhotovaultLogoProps> = ({ height }) => {
    const id = `pv${useId().replace(/[^a-zA-Z0-9]/g, "")}`;
    return (
        <svg
            height={height ?? 18}
            viewBox={pvWordmarkViewBox}
            xmlns="http://www.w3.org/2000/svg"
            role="img"
            aria-label="Photovault"
            // The markup is a constant from ./wordmark (no user input).
            dangerouslySetInnerHTML={{ __html: pvWordmarkSVGHTML(id) }}
        />
    );
};
