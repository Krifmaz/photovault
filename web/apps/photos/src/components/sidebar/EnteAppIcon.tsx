// PHOTOVAULT: strip - Ente's Auth and Locker logos are replaced by plain icons.
import KeyOutlinedIcon from "@mui/icons-material/KeyOutlined";
import LockOutlinedIcon from "@mui/icons-material/LockOutlined";
import type React from "react";

type EnteApp = "photos"; // PHOTOVAULT: strip

interface EnteAppIconProps {
    size?: number;
}

interface AppIconProps extends EnteAppIconProps {
    app: EnteApp;
}

const AppIcon: React.FC<AppIconProps> = ({ app, size = 24 }) => (
    <img
        src={`/images/app-icons/${app}.png`}
        srcSet={`/images/app-icons/2.0x/${app}.png 2x, /images/app-icons/3.0x/${app}.png 3x`}
        width={size}
        height={size}
        alt=""
        aria-hidden
        draggable={false}
        style={{ display: "block", objectFit: "contain" }}
    />
);

export const EntePhotosIcon: React.FC<EnteAppIconProps> = (props) => (
    <AppIcon app="photos" {...props} />
);

// PHOTOVAULT: strip - plain icons instead of Ente's Auth and Locker logos.
export const EnteAuthIcon: React.FC<EnteAppIconProps> = ({ size = 24 }) => (
    <KeyOutlinedIcon aria-hidden sx={{ fontSize: size, color: "text.muted" }} />
);

export const EnteLockerIcon: React.FC<EnteAppIconProps> = ({ size = 24 }) => (
    <LockOutlinedIcon
        aria-hidden
        sx={{ fontSize: size, color: "text.muted" }}
    />
);
